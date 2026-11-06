//! Registered-asset oracle and CPU wall timings; never needed by ordinary tests.
use super::*;
use std::{fs, path::PathBuf, time::Instant};

#[test]
#[ignore = "registered private quarry; run in the guarded adjacency batch"]
fn quarry_tables_and_timings() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var_os("RENDERLAB_PRIMARY_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let bytes = fs::read(manifest["asset"].as_str().unwrap()).unwrap();
    let asset = crate::asset::decode(&bytes, crate::asset::Limits::unlimited()).unwrap();
    drop(bytes);
    let meshes: Vec<_> = asset
        .meshes
        .iter()
        .flat_map(|m| &m.primitives)
        .map(|p| MeshData {
            vertices: &p.vertices,
            indices: &p.indices,
        })
        .collect();
    assert_eq!(meshes.len(), 12);
    assert_eq!(
        meshes.iter().map(|m| m.indices.len() / 3).sum::<usize>(),
        13_034_903
    );
    let mut expected: Option<Vec<Adjacency>> = None;
    let mut timings = vec![];
    for repeat in 0..8 {
        // Alternate order; each sample includes output allocation and scratch drop.
        for parallel in if repeat % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let started = Instant::now();
            let tables: Vec<_> = if parallel {
                meshes.par_iter().map(build).collect()
            } else {
                meshes.iter().map(legacy).collect()
            };
            let ms = started.elapsed().as_secs_f64() * 1000.;
            if let Some(expected) = &expected {
                assert_eq!(&tables, expected);
            } else {
                expected = Some(tables);
            }
            timings.push(serde_json::json!({"repeat":repeat,"parallel":parallel,"wall_ms":ms}));
        }
    }
    let mut thread_counts = vec![];
    for threads in [1, 2, 4, 8, 16] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let started = Instant::now();
        let tables: Vec<_> = pool.install(|| meshes.par_iter().map(build).collect());
        let ms = started.elapsed().as_secs_f64() * 1000.;
        assert_eq!(tables, *expected.as_ref().unwrap());
        thread_counts
            .push(serde_json::json!({"threads":threads,"wall_ms":ms,"byte_identical":true}));
    }
    let counts: Vec<_> = meshes.iter().zip(expected.unwrap()).enumerate().map(|(mesh,(data, table))| {
        serde_json::json!({"mesh":mesh,"triangles":data.indices.len()/3,
            "boundary_halfedges":table.boundary_halfedges,"ambiguous_halfedges":table.ambiguous_halfedges,
            "degenerate_halfedges":table.degenerate_halfedges})
    }).collect();
    let out = PathBuf::from(manifest["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    fs::write(
        out.join("cpu-adjacency.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "meshes":counts,"timings":timings,"thread_counts":thread_counts,
            "default_threads":rayon::current_num_threads(),"byte_identical":true,
        }))
        .unwrap(),
    )
    .unwrap();
}
