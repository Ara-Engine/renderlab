//! CPU evidence harness: independent upstream predicate and positional edge checks.
use crate::{Result, ensure};
use renderlab_asset::virtual_mesh::*;
use serde_json::json;
use std::collections::HashMap;

pub fn upstream_reference(mesh: &VirtualMesh, errors: &[f64], threshold: f64) -> Vec<ClusterId> {
    mesh.clusters
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            errors[c.owner.index()] > threshold
                && match c.refined {
                    None => true,
                    Some(g) => errors[g.index()] <= threshold,
                }
        })
        .map(|(i, _)| ClusterId(i as u32))
        .collect()
}
pub fn structural_cover(mesh: &VirtualMesh, cut: &Cut) -> Result<()> {
    let mut active = vec![false; mesh.groups.len()];
    for g in &cut.active_groups {
        ensure(g.index() < active.len(), "invalid cut group")?;
        ensure(!active[g.index()], "duplicate cut group")?;
        active[g.index()] = true;
    }
    ensure(
        mesh.terminal_groups().all(|g| active[g.index()]),
        "missing shallow/terminal cover",
    )?;
    let mut incoming = vec![vec![]; mesh.groups.len()];
    for (i, c) in mesh.clusters.iter().enumerate() {
        if let Some(g) = c.refined {
            incoming[g.index()].push(i);
        }
    }
    let mut selected = vec![false; mesh.clusters.len()];
    for g in mesh.terminal_groups() {
        for i in mesh.groups[g.index()].clusters.range(selected.len())? {
            selected[i] = true;
        }
    }
    for g in (0..mesh.groups.len())
        .rev()
        .filter(|&g| active[g] && !incoming[g].is_empty())
    {
        for &i in &incoming[g] {
            ensure(selected[i], "coarse shard absent/double replaced")?;
            selected[i] = false;
        }
        for i in mesh.groups[g].clusters.range(selected.len())? {
            ensure(!selected[i], "duplicate fine group coverage")?;
            selected[i] = true;
        }
    }
    let expected: Vec<_> = selected
        .iter()
        .enumerate()
        .filter(|(_, v)| **v)
        .map(|(i, _)| ClusterId(i as u32))
        .collect();
    ensure(
        cut.clusters == expected,
        "cut disagrees with exact-once group replay",
    )
}

/// Exact position-bit welding (signed zero canonicalized) ignores UV/material wedges only for topology.
/// Every undirected edge must occur exactly twice with opposite orientation. No epsilon weld.
pub fn closed_edges(
    mesh: &VirtualMesh,
    clusters: &[ClusterId],
    pages: &[PageGeometry],
) -> Result<usize> {
    type Point = [u32; 3];
    type Edge = (Point, Point);
    let mut edges: HashMap<Edge, (u32, i32)> = HashMap::new();
    for id in clusters {
        let c = &mesh.clusters[id.index()];
        let p = &pages[c.page.index()];
        for t in p.indices[c.indices.range(p.indices.len())?].chunks_exact(3) {
            let points: [[u32; 3]; 3] = std::array::from_fn(|i| {
                p.vertices[t[i] as usize]
                    .position
                    .map(|f| if f == 0. { 0 } else { f.to_bits() })
            });
            for k in 0..3 {
                let a = points[k];
                let b = points[(k + 1) % 3];
                ensure(a != b, "degenerate positional edge")?;
                let (edge, direction) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
                let v = edges.entry(edge).or_default();
                v.0 += 1;
                v.1 += direction;
            }
        }
    }
    let bad = edges
        .values()
        .filter(|&&(count, balance)| count != 2 || balance != 0)
        .count();
    ensure(
        bad == 0,
        format!(
            "closed cut has {bad} unpaired/nonmanifold/orientation edges of {}",
            edges.len()
        ),
    )?;
    Ok(edges.len())
}
pub fn projected_errors(mesh: &VirtualMesh, eye: [f64; 3], near: f64, scale: f64) -> Vec<f64> {
    let projection = 1080. * (1.0471975511965976_f64 / 2.).tan().recip() * 0.5;
    let mut errors: Vec<_> = mesh
        .groups
        .iter()
        .map(|g| {
            if g.simplified.error == f32::MAX {
                return f64::INFINITY;
            }
            let distance = g
                .simplified
                .center
                .iter()
                .zip(eye)
                .map(|(&p, e)| (f64::from(p) * scale - e).powi(2))
                .sum::<f64>()
                .sqrt();
            f64::from(g.simplified.error) * scale
                / (distance - f64::from(g.simplified.radius) * scale).max(near)
                * projection
        })
        .collect();
    // Roundoff-safe monotone envelope over the DAG; Unit B must use an equivalent conservative rule.
    for c in &mesh.clusters {
        if let Some(r) = c.refined {
            errors[c.owner.index()] = errors[c.owner.index()].max(errors[r.index()]);
        }
    }
    errors
}

pub fn prove(
    package: &Package,
    pages: &[PageGeometry],
    closed: bool,
    exhaustive: bool,
) -> Result<serde_json::Value> {
    let mut cuts = 0;
    let mut edge_checks = 0;
    let mut max_edges = 0;
    let mut constrained = 0;
    let mut maximum_scratch = 0;
    for mesh in &package.meshes {
        let available = vec![true; package.pages.len()];
        let unlimited = SelectionBudget {
            geometry_bytes: usize::MAX,
            max_candidates: usize::MAX,
            max_clusters: usize::MAX,
            max_scratch_bytes: 8 << 20,
        };
        for threshold in [0., 0.0001, 0.001, 0.01, 0.1, 1., 10.] {
            let errors: Vec<_> = mesh
                .groups
                .iter()
                .map(|g| f64::from(g.simplified.error))
                .collect();
            let s = select(mesh, &errors, threshold, &available, unlimited)?;
            ensure(
                s.cut.clusters == upstream_reference(mesh, &errors, threshold),
                "unlimited differs from upstream scalar rule",
            )?;
            structural_cover(mesh, &s.cut)?;
            cuts += 1;
            maximum_scratch = maximum_scratch.max(s.scratch_bound_bytes);
            if closed {
                max_edges = max_edges.max(closed_edges(mesh, &s.cut.clusters, pages)?);
                edge_checks += 1;
            }
        }
        for (eye, near, scale) in [
            ([4., 3., 5.], 0.05, 1.),
            ([0.8, 0.6, 1.5], 0.05, 1.),
            ([0., 0., 0.], 0.0001, 1.),
            ([4., 3., 5.], 0.05, 0.001),
            ([4., 3., 5.], 0.05, 1000.),
        ] {
            let errors = projected_errors(mesh, eye, near, scale);
            let s = select(mesh, &errors, 1., &available, unlimited)?;
            ensure(
                s.cut.clusters == upstream_reference(mesh, &errors, 1.),
                "camera errors differ from upstream rule",
            )?;
            structural_cover(mesh, &s.cut)?;
            cuts += 1;
            if closed {
                max_edges = max_edges.max(closed_edges(mesh, &s.cut.clusters, pages)?);
                edge_checks += 1;
            }
            for cap in [0, 1, 64, 1024] {
                let s = select(
                    mesh,
                    &errors,
                    1.,
                    &available,
                    SelectionBudget {
                        max_candidates: cap,
                        geometry_bytes: 8 << 20,
                        ..unlimited
                    },
                )?;
                structural_cover(mesh, &s.cut)?;
                constrained += 1;
                if closed {
                    max_edges = max_edges.max(closed_edges(mesh, &s.cut.clusters, pages)?);
                    edge_checks += 1;
                }
                let mut partial = available.clone();
                for (i, p) in partial.iter_mut().enumerate() {
                    *p = i % 3 != 0;
                }
                for terminal in mesh.terminal_groups() {
                    for c in &mesh.clusters[mesh.groups[terminal.index()]
                        .clusters
                        .range(mesh.clusters.len())?]
                    {
                        partial[c.page.index()] = true;
                    }
                }
                let s = select(
                    mesh,
                    &errors,
                    1.,
                    &partial,
                    SelectionBudget {
                        max_candidates: cap,
                        geometry_bytes: 8 << 20,
                        ..unlimited
                    },
                )?;
                structural_cover(mesh, &s.cut)?;
                constrained += 1;
                ensure(
                    s.demand.requested_pages
                        == s.demand
                            .required_pages
                            .iter()
                            .copied()
                            .filter(|p| !partial[p.index()])
                            .collect::<Vec<_>>(),
                    "required/requested closure disagreement",
                )?;
                if closed {
                    max_edges = max_edges.max(closed_edges(mesh, &s.cut.clusters, pages)?);
                    edge_checks += 1;
                }
            }
        }
        if exhaustive {
            ensure(
                mesh.groups.len() <= 20,
                "small exhaustive fixture unexpectedly exceeds 20 groups",
            )?;
            for mask in 0..1usize << mesh.groups.len() {
                let active: Vec<_> = (0..mesh.groups.len())
                    .map(|g| mask & (1 << g) != 0)
                    .collect();
                if !mesh.terminal_groups().all(|g| active[g.index()])
                    || !mesh.clusters.iter().all(|c| {
                        c.refined
                            .is_none_or(|r| !active[r.index()] || active[c.owner.index()])
                    })
                {
                    continue;
                }
                let errors: Vec<_> = active.iter().map(|&v| if v { 2. } else { 0. }).collect();
                let clusters = upstream_reference(mesh, &errors, 1.);
                let cut = Cut {
                    active_groups: active
                        .iter()
                        .enumerate()
                        .filter(|(_, v)| **v)
                        .map(|(g, _)| GroupId(g as u32))
                        .collect(),
                    clusters,
                    pages: vec![],
                };
                structural_cover(mesh, &cut)?;
                cuts += 1;
                if closed {
                    max_edges = max_edges.max(closed_edges(mesh, &cut.clusters, pages)?);
                    edge_checks += 1;
                }
            }
        }
    }
    Ok(
        json!({"unlimited_and_exhaustive_cuts":cuts,"constrained_selections":constrained,"closed_position_edge_checks":edge_checks,"maximum_closed_edge_count":max_edges,"selector_scratch_bound_bytes":maximum_scratch,"projection":"approximate perspective + monotone DAG envelope; uniform positive scales and near/interior cases; no pixel-quality guarantee"}),
    )
}
