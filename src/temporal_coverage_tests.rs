//! Held thin geometry, then immediate camera/object motion and a cut. Owned GPU lease.
use super::*;

#[test]
#[ignore = "owned GPU lease; RENDERLAB_THIN_OUT output directory"]
fn gpu_thin_coverage() {
    let out = PathBuf::from(std::env::var("RENDERLAB_THIN_OUT").unwrap());
    fs::create_dir_all(&out).unwrap();
    let quad = |r: &mut Renderer, points: [[f32; 3]; 4], colour: [f32; 4]| {
        let vertices: Vec<_> = points
            .into_iter()
            .map(|position| Vertex {
                position,
                normal: [0., 0., 1.],
                uv: [0.; 2],
                color: colour,
            })
            .collect();
        let mesh = r
            .try_upload_mesh(&MeshData {
                vertices: &vertices,
                indices: &[0, 1, 2, 0, 2, 3],
            })
            .unwrap()
            .unwrap();
        r.test_drain("thin fixture mesh");
        mesh
    };
    for path in ["nonrt", "rt"] {
        let mut r = if path == "rt" {
            renderer(true)
        } else {
            no_rt_renderer()
        };
        if path == "rt" {
            r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
        }
        r.set_shadows(&Shadows::default()).unwrap();
        r.set_indirect(IndirectSettings::off()).unwrap();
        r.set_reconstruction(Reconstruction::default()).unwrap();
        r.test_reload_standard(&standard_returning("input.color.xyz"))
            .unwrap();
        let background = quad(
            &mut r,
            [[-2., -2., 0.], [2., -2., 0.], [2., 2., 0.], [-2., 2., 0.]],
            [0.8, 0.8, 0.8, 1.],
        );
        let left = quad(
            &mut r,
            [
                [-0.92, -1., 0.1],
                [-0.908, -1., 0.1],
                [-0.075, 1.1, 0.1],
                [-0.079, 1.1, 0.1],
            ],
            [0.01, 0.025, 0.08, 1.],
        );
        let right = quad(
            &mut r,
            [
                [0.908, -1., 0.1],
                [0.92, -1., 0.1],
                [0.079, 1.1, 0.1],
                [0.075, 1.1, 0.1],
            ],
            [0.01, 0.025, 0.08, 1.],
        );
        let mut placements: Vec<_> = (0..3)
            .map(|_| {
                let placement = r.try_upload_instances(&[Mat4::IDENTITY]).unwrap().unwrap();
                r.test_drain("thin fixture placement");
                placement
            })
            .collect();
        r.test_drain("thin fixture placements");
        let color = target(&r, 160, 96);
        let camera = |x: f32| {
            Camera::look_at(
                Vec3::new(x, 0., 4.),
                Vec3::new(x, 0., 0.),
                Vec3::Y,
                0.8,
                0.05,
                100.,
            )
            .unwrap()
        };
        let meshes = [&background, &left, &right];
        let draw = |r: &mut Renderer, placements: &mut Vec<Instances>, x: f32| {
            let mut draws: Vec<_> = meshes
                .iter()
                .zip(placements.iter_mut())
                .map(|(m, p)| Draw::new(m, p))
                .collect();
            render(r, &color, &camera(x), &mut draws);
        };
        draw(&mut r, &mut placements, 0.);
        r.test_drain("thin pipeline warmup");
        let mut rows = vec![];
        let mut reference = vec![0f32; 160 * 96 * 4];
        let mut baseline_peak = 0f32;
        for mode in ["off", "unorm", "cycle"] {
            let raw = include_str!("../shaders/temporal_resolve.wgsl");
            let mut resolve = match mode {
                "off" => raw.to_owned(),
                _ => temporal_reconstruction::coverage_resolve(raw, mode == "cycle"),
            };
            // Shader-only iteration reuses this binary; the final compiled
            // candidate still validates the exact embedded production sources.
            if mode == "cycle" {
                if let Ok(path) = std::env::var("RENDERLAB_THIN_CYCLE_SOURCE") {
                    resolve = resolve.replace(
                        include_str!("../shaders/temporal_coverage_cycle.wgsl"),
                        &fs::read_to_string(path).unwrap(),
                    );
                }
            }
            let body = format!(
                "{}\n{resolve}\n{}",
                include_str!("../shaders/object_motion.wgsl"),
                temporal_reconstruction::zones_source("tr1.zones")
            );
            r.test_resolve_source(Some(&body));
            r.cut_reconstruction();
            r.view.reconstruction_index = 0;
            r.cut_post_motion();
            for p in &mut placements {
                p.set(0, Mat4::IDENTITY).unwrap();
            }
            let mut previous = None::<Vec<f32>>;
            let mut before_unrelated_mover = None::<Vec<f32>>;
            let mut peaks = vec![0f32; 160 * 96];
            for frame in 0..712 {
                let x = if (408..424).contains(&frame) {
                    (frame - 407) as f32 * 0.004
                } else if frame >= 424 {
                    -0.1
                } else {
                    0.
                };
                if frame == 424 {
                    r.cut_reconstruction();
                    r.cut_post_motion();
                }
                if frame == 432 {
                    placements[1]
                        .set(0, Mat4::from_translation(Vec3::new(0.2, 0., 0.)))
                        .unwrap();
                }
                if frame == 700 {
                    placements[1].set(0, Mat4::IDENTITY).unwrap();
                }
                draw(&mut r, &mut placements, x);
                r.test_drain("thin frame");
                if mode == "off" && frame < 256 {
                    let samples = read_texture(
                        &r,
                        r.view.reconstruction_state.as_ref().unwrap().test_samples(),
                    );
                    for (mean, b) in reference.iter_mut().zip(samples.chunks_exact(2)) {
                        *mean += decode_f16(u16::from_le_bytes(b.try_into().unwrap())) / 256.;
                    }
                }
                if (400..440).contains(&frame) || (694..710).contains(&frame) {
                    let hdr: Vec<_> =
                        read_texture(&r, &r.view.hdr.as_ref().unwrap().resolved.texture)
                            .chunks_exact(2)
                            .map(|b| decode_f16(u16::from_le_bytes(b.try_into().unwrap())))
                            .collect();
                    if mode == "cycle" && (frame == 699 || frame == 701) {
                        // The global motion box also clamps colours in main. Test
                        // geometric coverage directly, without mistaking that for
                        // invalidation of the unchanged right rail's statistic.
                        let words = read_texture(
                            &r,
                            r.view.reconstruction_state.as_ref().unwrap().test_layers(),
                        );
                        let coverage: Vec<_> = words
                            .chunks_exact(16)
                            .map(|b| {
                                let flags = u32::from_le_bytes(b[..4].try_into().unwrap());
                                if flags == u32::MAX {
                                    return -1.;
                                }
                                let word = u32::from_le_bytes(b[12..16].try_into().unwrap());
                                let c = if flags & 0x2000000 != 0 {
                                    decode_f16(word as u16)
                                        / decode_f16((word >> 16) as u16).max(1.)
                                } else {
                                    (word & 0xffff) as f32 / 65535.
                                };
                                if flags & 0x1000000 != 0 { c } else { 1. - c }
                            })
                            .collect();
                        if frame == 699 {
                            before_unrelated_mover = Some(coverage);
                        } else {
                            let before = before_unrelated_mover.as_ref().unwrap();
                            let kept: Vec<_> = coverage
                                .iter()
                                .zip(before)
                                .enumerate()
                                .filter(|(i, (a, b))| i % 160 >= 92 && **a >= 0. && **b >= 0.)
                                .collect();
                            let max_right = kept
                                .iter()
                                .map(|(_, (a, b))| (*a - *b).abs())
                                .fold(0f32, f32::max);
                            eprintln!(
                                "THIN_UNRELATED_MOVER {path} pairs={} max_right_coverage_delta={max_right}",
                                kept.len()
                            );
                            assert!(kept.len() > 8, "fixture must retain right-rail pairs");
                            assert_eq!(
                                max_right, 0.,
                                "unrelated mover must preserve settled coverage"
                            );
                        }
                    }
                    if frame < 408 {
                        if let Some(prev) = previous.as_ref() {
                            for (i, (a, b)) in
                                hdr.chunks_exact(4).zip(prev.chunks_exact(4)).enumerate()
                            {
                                peaks[i] = peaks[i].max(
                                    ((a[0] - b[0]) * 0.2126
                                        + (a[1] - b[1]) * 0.7152
                                        + (a[2] - b[2]) * 0.0722)
                                        .abs(),
                                );
                            }
                        }
                    }
                    fs::write(
                        out.join(format!("{path}-{mode}-{frame}.f32")),
                        hdr.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>(),
                    )
                    .unwrap();
                    if frame == 407 {
                        let error = hdr
                            .chunks_exact(4)
                            .zip(reference.chunks_exact(4))
                            .map(|(a, b)| {
                                ((a[0] - b[0]) * 0.2126
                                    + (a[1] - b[1]) * 0.7152
                                    + (a[2] - b[2]) * 0.0722)
                                    .abs()
                            })
                            .sum::<f32>()
                            / (160. * 96.);
                        eprintln!("THIN_REFERENCE {path} {mode} mean_hdr_luma_error={error}");
                    }
                    previous = Some(hdr);
                }
            }
            let peak = peaks.iter().copied().fold(0f32, f32::max);
            if mode == "off" {
                baseline_peak = peak;
            }
            if mode == "cycle" {
                assert!(
                    baseline_peak > 0.001,
                    "fixture must reproduce held coverage crawl"
                );
                assert!(
                    peak < baseline_peak * 0.1,
                    "cycle must remove held crawl: {peak} vs {baseline_peak}"
                );
            }
            rows.push(serde_json::json!({"path":path,"mode":mode,"peak_hdr_luma":peak,"pixels_over_001":peaks.iter().filter(|&&p|p>=0.01).count()}));
            eprintln!("THIN {}", rows.last().unwrap());
        }
        fs::write(
            out.join(format!("{path}.json")),
            serde_json::to_vec_pretty(&rows).unwrap(),
        )
        .unwrap();
        assert!(r.fault.get().is_none());
    }
}
