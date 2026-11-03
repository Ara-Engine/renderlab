use super::*;
use asset::virtual_mesh::{GroupId, PageId, VirtualMesh};

pub fn verify_cover(
    asset: &VirtualAsset,
    scene: &VirtualScene,
    options: StreamOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let s = asset.stats();
    if s.geometry_bytes > options.geometry_bytes
        || s.geometry_peak > options.geometry_bytes
        || s.metadata_bytes + s.scratch_bound_bytes > options.metadata_bytes
        || s.cpu_ready_bytes > s.cpu_io_bound_bytes
        || s.texture_bytes > options.texture_bytes
        || s.candidates > options.max_candidates
        || s.cut_clusters > options.max_clusters
    {
        return Err("replay resource/work budget exceeded".into());
    }
    for (placement, selection) in asset.package().placements.iter().zip(scene.selections()) {
        let mesh = &asset.package().meshes[placement.mesh as usize];
        let active: BTreeSet<_> = selection.cut.active_groups.iter().copied().collect();
        if active.len() != selection.cut.active_groups.len()
            || mesh.terminal_groups().any(|g| !active.contains(&g))
            || mesh.clusters.iter().any(|c| {
                c.refined.is_some_and(|r| active.contains(&r)) && !active.contains(&c.owner)
            })
        {
            return Err("replay cut has incomplete replacement closure".into());
        }
        let expected: Vec<_> = mesh
            .clusters
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                active.contains(&c.owner) && c.refined.is_none_or(|r| !active.contains(&r))
            })
            .map(|(i, _)| asset::virtual_mesh::ClusterId(i as u32))
            .collect();
        let pages: BTreeSet<_> = expected
            .iter()
            .map(|c| mesh.clusters[c.index()].page)
            .collect();
        if expected != selection.cut.clusters
            || pages.iter().copied().collect::<Vec<_>>() != selection.cut.pages
            || pages
                .iter()
                .any(|&p| asset.status(p) != Some(PageStatus::Resident))
        {
            return Err("replay cut is not the complete resident cluster cover".into());
        }
    }
    Ok(())
}

pub fn groups(
    path: &std::path::Path,
    asset: &VirtualAsset,
    scene: &VirtualScene,
    pose: &cameras::Pose,
    options: StreamOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut output = BufWriter::new(fs::File::create(path)?);
    writeln!(
        output,
        "placement,group,depth,material,error,active,selected_clusters,selected_indices,missing,queued,ready,uploading,resident,retiring,failed,in_requested,in_required,independently_publishable,transition_fits,transition_pages,bbox_left,bbox_top,bbox_right,bbox_bottom"
    )?;
    let mut protected = BTreeSet::new();
    for mesh in &asset.package().meshes {
        for c in &mesh.clusters {
            if mesh.groups[c.owner.index()].simplified.error == f32::MAX {
                protected.insert(c.page);
            }
        }
    }
    for s in scene.selections() {
        protected.extend(&s.cut.pages);
        protected.extend(&s.demand.required_pages);
    }
    for (slot, (placement, selection)) in asset
        .package()
        .placements
        .iter()
        .zip(scene.selections())
        .enumerate()
    {
        let mesh = &asset.package().meshes[placement.mesh as usize];
        let model = Mat4::from_cols_array_2d(&placement.transform);
        let camera = pose.camera()?;
        let errors = project_errors(mesh, &camera, model, pose.height)?;
        for (id, group) in mesh.groups.iter().enumerate() {
            let mut statuses = [0usize; 7];
            let mut pages = BTreeSet::new();
            for c in &mesh.clusters[group.clusters.range(mesh.clusters.len())?] {
                pages.insert(c.page);
            }
            for &p in &pages {
                statuses[asset.status(p).unwrap() as usize] += 1;
            }
            let selected: Vec<_> = selection
                .cut
                .clusters
                .iter()
                .map(|c| &mesh.clusters[c.index()])
                .filter(|c| c.owner.index() == id)
                .collect();
            let independent = closure_pages(mesh, &selection.cut.active_groups, GroupId(id as u32));
            let publishable = independent
                .iter()
                .all(|&p| asset.status(p) == Some(PageStatus::Resident));
            let charge = protected.union(&independent).count();
            let fits = charge * asset::virtual_mesh::PAGE_BYTES
                <= options.geometry_bytes - asset::virtual_mesh::PAGE_BYTES
                && closure_clusters(mesh, &selection.cut.active_groups, GroupId(id as u32))
                    <= options.max_clusters;
            let bbox = screen_bounds(group.simplified, model, &camera, pose.width, pose.height);
            writeln!(
                output,
                "{slot},{id},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{publishable},{fits},{charge},{},{},{},{}",
                group.depth,
                mesh.clusters[group.clusters.offset as usize].material,
                errors[id],
                selection
                    .cut
                    .active_groups
                    .binary_search(&GroupId(id as u32))
                    .is_ok(),
                selected.len(),
                selected.iter().map(|c| c.indices.count).sum::<u32>(),
                statuses[0],
                statuses[1],
                statuses[2],
                statuses[3],
                statuses[4],
                statuses[5],
                statuses[6],
                pages
                    .iter()
                    .any(|p| selection.demand.requested_pages.binary_search(p).is_ok()),
                pages
                    .iter()
                    .any(|p| selection.demand.required_pages.binary_search(p).is_ok()),
                bbox[0],
                bbox[1],
                bbox[2],
                bbox[3]
            )?;
        }
    }
    output.flush()?;
    Ok(())
}

fn closure(mesh: &VirtualMesh, published: &[GroupId], seed: GroupId) -> BTreeSet<GroupId> {
    let mut active: BTreeSet<_> = published.iter().copied().chain([seed]).collect();
    loop {
        let before = active.len();
        for c in mesh.clusters.iter().rev() {
            if c.refined.is_some_and(|r| active.contains(&r)) {
                active.insert(c.owner);
            }
        }
        if active.len() == before {
            return active;
        }
    }
}
fn closure_pages(mesh: &VirtualMesh, published: &[GroupId], seed: GroupId) -> BTreeSet<PageId> {
    let active = closure(mesh, published, seed);
    mesh.clusters
        .iter()
        .filter(|c| active.contains(&c.owner) && c.refined.is_none_or(|r| !active.contains(&r)))
        .map(|c| c.page)
        .collect()
}
fn closure_clusters(mesh: &VirtualMesh, published: &[GroupId], seed: GroupId) -> usize {
    let active = closure(mesh, published, seed);
    mesh.clusters
        .iter()
        .filter(|c| active.contains(&c.owner) && c.refined.is_none_or(|r| !active.contains(&r)))
        .count()
}

/// Conservative projected sphere rectangle. Near-intersecting spheres cover the viewport.
fn screen_bounds(
    b: asset::virtual_mesh::Bounds,
    model: Mat4,
    camera: &Camera,
    width: u32,
    height: u32,
) -> [f64; 4] {
    let linear = glam::DMat3::from_mat4(model.as_dmat4()).to_cols_array();
    let one = (0..3)
        .map(|c| (0..3).map(|r| linear[c * 3 + r].abs()).sum::<f64>())
        .fold(0., f64::max);
    let inf = (0..3)
        .map(|r| (0..3).map(|c| linear[c * 3 + r].abs()).sum::<f64>())
        .fold(0., f64::max);
    let radius = f64::from(b.radius) * (one * inf).sqrt();
    let c = (camera.view() * model)
        .as_dmat4()
        .transform_point3(glam::DVec3::from_array(b.center.map(f64::from)));
    if -c.z - radius <= f64::from(camera.near()) {
        return [0., 0., width as f64, height as f64];
    }
    let focal = height as f64 * 0.5 / (f64::from(camera.fov_y()) * 0.5).tan();
    let mut bbox = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for x in [-radius, radius] {
        for y in [-radius, radius] {
            for z in [-radius, radius] {
                let px = width as f64 * 0.5 + (c.x + x) * focal / (-c.z - z);
                let py = height as f64 * 0.5 - (c.y + y) * focal / (-c.z - z);
                bbox[0] = bbox[0].min(px);
                bbox[1] = bbox[1].min(py);
                bbox[2] = bbox[2].max(px);
                bbox[3] = bbox[3].max(py);
            }
        }
    }
    bbox
}

pub struct PageLog {
    output: BufWriter<fs::File>,
    previous: Vec<(PageStatus, bool, bool)>,
    pub overhead: Duration,
}
impl PageLog {
    pub fn new(path: &std::path::Path, count: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let mut output = BufWriter::new(fs::File::create(path)?);
        writeln!(output, "tick,frame,page,before,after,requested,required")?;
        Ok(Self {
            output,
            previous: vec![(PageStatus::Missing, false, false); count],
            overhead: Duration::ZERO,
        })
    }
    pub fn sample(
        &mut self,
        tick: usize,
        frame: usize,
        asset: &VirtualAsset,
        scene: &VirtualScene,
    ) -> std::io::Result<()> {
        let start = Instant::now();
        for (p, previous) in self.previous.iter_mut().enumerate() {
            let id = PageId(p as u32);
            let now = (
                asset.status(id).unwrap(),
                scene
                    .selections()
                    .iter()
                    .any(|s| s.demand.requested_pages.binary_search(&id).is_ok()),
                scene
                    .selections()
                    .iter()
                    .any(|s| s.demand.required_pages.binary_search(&id).is_ok()),
            );
            if now != *previous {
                writeln!(
                    self.output,
                    "{tick},{frame},{p},{:?},{:?},{},{}",
                    previous.0, now.0, now.1, now.2
                )?;
                *previous = now;
            }
        }
        self.overhead += start.elapsed();
        Ok(())
    }
}
