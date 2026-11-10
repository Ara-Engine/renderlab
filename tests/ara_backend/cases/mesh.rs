//! Indexed, instanced mesh transforms and diagnostic shading against scalar raster.
use super::*;
use ara_gpu::{sampler::SamplerDesc, texture::SampleCount};
use renderlab::backend_ara::{
    mesh::{DiagnosticCamera, DiagnosticInputs, DiagnosticMaterial, DiagnosticMesh, MeshGeometry},
    post_surface::RasterDepth,
};
#[test]
fn indexed_instanced_mesh_native_replay_against_cpu() -> Result<()> {
    let mut fixture = mesh_reference::fixture();
    let path = root("mesh");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let pass = DiagnosticMesh::new(
        &mut gpu,
        &cache,
        Format::Rgba32Float,
        [SamplerDesc::default(); 2],
    )?;
    let geometry = MeshGeometry::new(
        &mut gpu,
        &fixture.vertices,
        &fixture.indices,
        &fixture.slots,
        &fixture.models,
    )?;
    let base = Texture::new(
        &mut gpu,
        fixture.base.size,
        Format::Rgba16Float,
        false,
        false,
    )?;
    let mr = Texture::new(&mut gpu, fixture.mr.size, Format::Rgba16Float, false, false)?;
    let output = Texture::new(&mut gpu, fixture.viewport, Format::Rgba32Float, false, true)?;
    let depth = RasterDepth::new(&mut gpu, fixture.viewport, SampleCount::One)?;
    let mut graph = Graph::default();
    geometry.import(&mut graph);
    let mut rec = gpu.begin(Queue::Graphics);
    let mut retire = vec![
        upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            base.image,
            &half_bytes(&fixture.base.pixels),
        )?,
        upload(
            &mut gpu,
            &mut graph,
            &mut rec,
            mr.image,
            &half_bytes(&fixture.mr.pixels),
        )?,
    ];
    let mut cameras = Vec::new();
    let mut materials = Vec::new();
    let mut snapshots = Vec::new();
    let mut last = None;
    for pose in 0..2 {
        if pose == 1 {
            fixture.models[1] = renderlab::Mat4::from_scale(renderlab::Vec3::splat(0.8));
            fixture.normals[1] = mesh_reference::normal_record(fixture.models[1]);
            geometry.update_models(&mut graph, &mut rec, 1, &fixture.models[1..2])?;
        }
        for double_sided in [false, true] {
            fixture.material.double_sided = double_sided;
            let material = DiagnosticMaterial::new(
                &mut gpu,
                fixture.material.base_color,
                fixture.material.metallic,
                fixture.material.roughness,
                double_sided,
            )?;
            // The backend reuses Renderlab's material writer; compare its actual ABI bytes.
            ensure!(
                gpu.read(material.record.buffer)?
                    == bytemuck::cast_slice::<u32, u8>(&fixture.material.words()),
                "material ABI differs"
            );
            graph.import(Resource::Buffer(material.record.buffer), Access::HostWrite);
            for mode in 0..4 {
                let camera = DiagnosticCamera::new(&mut gpu, fixture.view_projection, mode)?;
                graph.import(Resource::Buffer(camera.record.buffer), Access::HostWrite);
                pass.record(
                    &mut graph,
                    &mut rec,
                    &DiagnosticInputs {
                        geometry: &geometry,
                        camera: &camera,
                        material: &material,
                        base: &base,
                        metallic_roughness: &mr,
                    },
                    &output,
                    &depth,
                    fixture.background,
                )?;
                let read = renderlab::backend_ara::transfer::readback(
                    &mut gpu,
                    &mut graph,
                    &mut rec,
                    output.image,
                    u64::from(fixture.viewport[0] * fixture.viewport[1] * 16),
                )?;
                let depth_read = renderlab::backend_ara::transfer::readback(
                    &mut gpu,
                    &mut graph,
                    &mut rec,
                    depth.image(),
                    u64::from(fixture.viewport[0] * fixture.viewport[1] * 4),
                )?;
                snapshots.push((
                    pose,
                    double_sided,
                    mode,
                    read,
                    depth_read,
                    mesh_reference::rasterize(&fixture, mode),
                ));
                cameras.push(camera);
            }
            materials.push(material);
        }
        last = Some(gpu.submit(rec.finish(), &[])?);
        rec = gpu.begin(Queue::Graphics);
    }
    // Both geometry poses and all snapshots are submitted before this first wait.
    gpu.wait(last.expect("two geometry submissions"))?;
    for (pose, sided, mode, read, depth_read, expected) in snapshots {
        let bytes = gpu.read(read)?;
        let actual: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .collect();
        let mut max = 0f32;
        let mut coverage = 0;
        for (i, (a, b)) in actual
            .as_chunks::<4>()
            .0
            .iter()
            .zip(&expected.image.pixels)
            .enumerate()
        {
            let actual_hit = *a != fixture.background;
            let expected_hit = *b != fixture.background;
            if actual_hit != expected_hit {
                coverage += 1;
            }
            for (a, b) in a.iter().zip(b) {
                ensure!(a.is_finite(), "nonfinite mesh at {i}");
                max = max.max((a - b).abs());
            }
        }
        if std::env::var_os("ARA_BACKEND_CAPTURE").is_some() && mode == 0 {
            // Inspection artifact only: convert the measured linear GPU readback
            // to display bytes on the CPU; shader comparisons stay in linear float.
            let raw = reference::Image {
                size: fixture.viewport,
                pixels: actual.as_chunks::<4>().0.to_vec(),
            };
            let display = reference::display(&raw);
            let bytes: Vec<u8> = display
                .pixels
                .iter()
                .flatten()
                .map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
                .collect();
            let mut encoder = png::Encoder::new(
                std::fs::File::create(path.join(format!("mesh-pose-{pose}-sided-{sided}.png")))?,
                fixture.viewport[0],
                fixture.viewport[1],
            );
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
            encoder.write_header()?.write_image_data(&bytes)?;
            println!("mesh inspection artifact: {}", path.display());
        }
        println!(
            "mesh pose {pose} double_sided={sided} mode {mode}: max error {max}, coverage mismatches {coverage}"
        );
        ensure!(coverage == 0, "mesh coverage differs");
        ensure!(max <= 0.001, "mesh shading differs by {max}");
        let bytes = gpu.read(depth_read)?;
        let mut depth_error = 0f32;
        for (a, b) in bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .zip(expected.depth)
        {
            ensure!(a.is_finite(), "nonfinite mesh depth");
            depth_error = depth_error.max((a - b).abs());
        }
        println!("mesh depth: max error {depth_error}");
        ensure!(
            depth_error <= 1e-6,
            "mesh reversed depth differs by {depth_error}"
        );
        retire.extend([read, depth_read]);
    }
    for r in cameras {
        r.retire(&mut gpu, &mut graph)?;
    }
    for r in materials {
        r.retire(&mut gpu, &mut graph)?;
    }
    geometry.retire(&mut gpu, &mut graph)?;
    depth.retire(&mut gpu, &mut graph)?;
    for t in [base, mr, output] {
        gpu.retire(Resource::Image(t.image))?;
    }
    for b in retire {
        gpu.retire(Resource::Buffer(b))?;
    }
    pass.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
