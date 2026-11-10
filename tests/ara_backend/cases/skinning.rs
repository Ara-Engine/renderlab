//! Native skinning replay through the public backend encoder.
use super::*;
#[test]
fn skinned_streams_history_and_cluster_copies_native_replay() -> Result<()> {
    use renderlab::backend_ara::skinning::{SkinningBuffers, SkinningDesc, SkinningPasses};
    let f = skinning_reference::fixture();
    let path = root("skinning");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let passes = SkinningPasses::new(&mut gpu, &cache)?;
    let buffers = SkinningBuffers::new(
        &mut gpu,
        SkinningDesc {
            vertices: &f.vertices,
            influences: &f.influences,
            tangents: Some(&f.tangents),
            mappings: &f.mappings,
            cluster_words: &f.cluster_words,
            acceleration_usage: vk::BufferUsageFlags::empty(),
        },
        &f.palettes[0],
    )?;
    let mut graph = Graph::default();
    buffers.import(&mut graph);
    let mut snapshots = Vec::new();
    let mut points = Vec::new();
    for frame in 0..2 {
        let mut rec = gpu.begin(Queue::Graphics);
        if frame == 1 {
            buffers.update_palette(&mut graph, &mut rec, &f.palettes[frame], false)?;
        }
        passes.record(&mut graph, &mut rec, Pass::SkinVertices, &buffers)?;
        passes.record(&mut graph, &mut rec, Pass::SkinClusters, &buffers)?;
        let mut reads = Vec::new();
        for (buffer, len) in [
            (buffers.vertices(), f.vertices.len() * 48),
            (buffers.previous(), f.vertices.len() * 48 + 16),
            (buffers.tangents(), f.tangents.len() * 16),
            (buffers.clusters(), f.cluster_words.len() * 4),
        ] {
            reads.push(renderlab::backend_ara::transfer::readback_buffer(
                &mut gpu, &mut graph, &mut rec, buffer, 0, len as u64,
            )?);
        }
        points.push(gpu.submit(rec.finish(), &[])?);
        snapshots.push(reads);
    }
    // Both poses and their GPU snapshots are queued before the first host wait.
    gpu.wait(points[1])?;
    let mut last_vertices = Vec::new();
    for (frame, reads) in snapshots.into_iter().enumerate() {
        let expected = skinning_reference::deform(
            &f.vertices,
            &f.influences,
            &f.palettes[frame],
            Some(&f.tangents),
        );
        let decode = |bytes: Vec<u8>| -> Vec<f32> {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|w| f32::from_le_bytes(*w))
                .collect()
        };
        let vertices = decode(gpu.read(reads[0])?);
        let previous = decode(gpu.read(reads[1])?);
        let tangents = decode(gpu.read(reads[2])?);
        let mut maximum = 0f32;
        for (actual, wanted) in vertices
            .iter()
            .zip(expected.vertices.iter().flatten())
            .chain(tangents.iter().zip(expected.tangents.iter().flatten()))
        {
            ensure!(actual.is_finite(), "nonfinite skinned stream");
            maximum = maximum.max((actual - wanted).abs());
        }
        ensure!(maximum <= 2e-5, "deformation differs by {maximum}");
        for (index, source) in f.vertices.iter().enumerate() {
            let untouched: Vec<_> = source.uv.into_iter().chain(source.color).collect();
            ensure!(
                vertices[index * 12 + 6..index * 12 + 12] == untouched,
                "source attributes changed"
            );
        }
        let wanted_previous = if frame == 0 {
            &vertices
        } else {
            &last_vertices
        };
        ensure!(
            &previous[..vertices.len()] == wanted_previous,
            "previous pose is not last submitted geometry"
        );
        ensure!(
            previous[vertices.len()..].iter().all(|v| v.to_bits() == 0),
            "history padding changed"
        );
        let mut cluster = f.cluster_words.clone();
        for &[dst, source] in &f.mappings {
            for c in 0..3 {
                cluster[dst as usize + c] = vertices[source as usize * 12 + c].to_bits();
            }
        }
        let actual: Vec<_> = gpu
            .read(reads[3])?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| u32::from_le_bytes(*w))
            .collect();
        ensure!(
            actual == cluster,
            "cluster position or sentinel copy differs"
        );
        println!(
            "skin pose {frame}: max deformation error {maximum}; previous pose and all cluster/sentinel words exact"
        );
        last_vertices = vertices;
        for read in reads {
            gpu.retire(Resource::Buffer(read))?;
            graph.forget(Resource::Buffer(read));
        }
    }
    buffers.retire(&mut gpu, &mut graph)?;
    passes.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
