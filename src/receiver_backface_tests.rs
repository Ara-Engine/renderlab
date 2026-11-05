//! One bounded alternative screen; production shaders and adjacency stay intact.
use super::super::film_accumulation_tests as t1;
use super::*;
use sha2::{Digest, Sha256};

fn retry_source(source: &str) -> String {
    let source = edit(
        source,
        (
            "return PrimaryTrace(vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics)),point,v);",
            "return primary_backface_retry(PrimaryTrace(vec4<u32>(hit.instance_custom_data+1u,hit.primitive_index,bitcast<vec2<u32>>(hit.barycentrics)),point,v),ray.near,direction,hit.t,hit.front_face);",
        ),
    );
    // Keep the concave-only inset and its exact geometry/arithmetic. Acquire its
    // neighbour using the existing fallback probe, so this variant needs no table.
    let source = edit(
        &source,
        (
            "for(var probe=0u;probe<2u;probe+=1u)",
            "for(var probe=1u;probe<2u;probe+=1u)",
        ),
    );
    let source = edit(
        &source,
        (
            "let neighbour=primary_edge_neighbour(batch,trace.record.y,edge);",
            "let neighbour=0xffffffffu;",
        ),
    );
    format!(
        "{source}\n{}",
        include_str!("../support/receiver_backface_retry.wgsl")
    )
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn backface_retry_sources_validate() {
    for source in primary_ray_gpu::shader_sources(SOURCE).unwrap() {
        validate_wgsl(&retry_source(&source));
    }
    let footprint = retry_source(&t1::footprint_source());
    validate_wgsl(&footprint);
    validate_wgsl(&witness_source(
        &footprint,
        &serde_json::json!([{"pose":0,"origin_bits":[0,0,0],"direction_bits":[0,0,1065353216]}]),
    ));
    validate_wgsl(&difference_source());
}

fn image_buffers(r: &mut Renderer, soft: bool) -> Vec<Vec<u8>> {
    let mut buffers = vec![hit_record(r), capture(r)];
    if soft {
        buffers.extend(receivers(r));
    }
    buffers
}

fn changed_pixels(a: &[u8], b: &[u8], stride: usize) -> Vec<usize> {
    a.chunks_exact(stride)
        .zip(b.chunks_exact(stride))
        .enumerate()
        .filter_map(|(i, (a, b))| (a != b).then_some(i))
        .collect()
}

fn witness_source(footprint: &str, rays: &serde_json::Value) -> String {
    let mut cases = String::new();
    for ray in rays.as_array().unwrap() {
        let vector = |key: &str| {
            format!(
                "bitcast<vec3<f32>>(vec3<u32>({}))",
                ray[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| format!("{}u", v.as_u64().unwrap()))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        cases.push_str(&format!(
            "case {}u: {{return array<vec3<f32>,2>({},{});}}\n",
            ray["pose"],
            vector("origin_bits"),
            vector("direction_bits")
        ));
    }
    let entry = footprint
        .find("@compute @workgroup_size(8,4) fn film_footprint(")
        .unwrap();
    let source = edit(
        &footprint[..entry],
        (
            "let ray=primary_ray(pixel,size);let delta=ray.far-ray.near;let distance=length(delta);\n    let direction=delta/distance;",
            "let inputs=backface_witness_ray(pixel.x);let direction=inputs[1];let distance=100.0;let ray=PrimaryRay(inputs[0],vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(0.0));",
        ),
    );
    format!(
        "{source}\n{}\nfn backface_witness_ray(index:u32)->array<vec3<f32>,2> {{switch index {{{cases} default: {{return array<vec3<f32>,2>();}}}}}}\n{}",
        include_str!("../support/receiver_backface_retry.wgsl"),
        r#"
@compute @workgroup_size(8,4) fn film_footprint(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy!=vec2<u32>(0u)) {return;}
    let index=footprint_params.y;let ray=backface_witness_ray(index);
    let raw=primary_raw_closest(vec2<u32>(index,0u),vec2<u32>(1920u,1080u));
    let fixed=primary_repair(raw,ray[0],ray[1]);
    atomicStore(&footprint_sums[index].accepted,raw.record.x);
    atomicStore(&footprint_sums[index].visible,raw.record.y);
    atomicStore(&footprint_sums[index].opposite,raw.record.z);
    atomicStore(&footprint_sums[index].corner,raw.record.w);
    atomicStore(&footprint_sums[index].edge,fixed.record.x);
    atomicStore(&footprint_sums[index].interior,fixed.record.y);
    atomicStore(&footprint_sums[index].nearest,fixed.record.z);
    atomicStore(&footprint_sums[index].rejected,fixed.record.w);
}
"#
    )
}

fn micro_targets(m: &serde_json::Value, pose: usize, surface: &[u8], camera: &Camera) -> Vec<u8> {
    let inverse = camera.view().inverse();
    let eye = inverse.w_axis;
    let forward = -inverse.z_axis;
    let mut targets = vec![];
    for p in m["footprint"][pose.to_string()].as_array().unwrap() {
        let x = p[0].as_u64().unwrap() as u32;
        let y = p[1].as_u64().unwrap() as u32;
        targets.extend([x, y, 1920, 1080].iter().flat_map(|v| v.to_le_bytes()));
        let i = (x + y * 1920) as usize * 8;
        targets.extend_from_slice(&surface[i..i + 8]);
        targets.extend([1u32, 0].iter().flat_map(|v| v.to_le_bytes()));
        targets.extend(eye.to_array().iter().flat_map(|v| v.to_le_bytes()));
        targets.extend(forward.to_array().iter().flat_map(|v| v.to_le_bytes()));
        targets.extend(
            [
                p[3].as_f64().unwrap() as f32,
                p[4].as_f64().unwrap() as f32,
                p[5].as_f64().unwrap() as f32,
                0.,
            ]
            .iter()
            .flat_map(|v| v.to_le_bytes()),
        );
    }
    targets
}

fn difference_source() -> String {
    let footprint = t1::footprint_source();
    let end = footprint
        .find("@compute @workgroup_size(8,4) fn film_footprint(")
        .unwrap();
    let source = edit(
        &footprint[..end],
        (
            "let v=getCommittedHitVertexPositions(&query);",
            "difference_t=hit.t;difference_front=u32(hit.front_face);\n    let v=getCommittedHitVertexPositions(&query);",
        ),
    );
    format!(
        "{source}\n{}",
        include_str!("../support/receiver_difference_rays.wgsl")
    )
}

#[test]
#[ignore = "owned GPU lease; second and final screen, seven exact-ray classifications"]
fn gpu_backface_retry_differences() {
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let (mut asset, _) = load_quarry(&m, &mut r);
    let color = target(&r, 1920, 1080);
    let source = difference_source();
    fs::write(out.join("difference-rays.wgsl"), &source).unwrap();
    let mut rays = vec![];
    for (pose, camera) in t1::t1_cameras(&m).iter().enumerate() {
        let diff: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join(format!("diff-{pose}-hard.json"))).unwrap())
                .unwrap();
        let points = diff
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["buffer"] == "hit")
            .unwrap()["first_pixels"]
            .as_array()
            .unwrap();
        if points.is_empty() {
            continue;
        }
        traced_frame(&mut r, &color, camera, &mut asset, true);
        let mut targets = vec![];
        for point in points {
            let mut target = vec![0u8; 80];
            for (i, word) in [
                point[0].as_u64().unwrap() as u32,
                point[1].as_u64().unwrap() as u32,
                1920,
                1080,
            ]
            .iter()
            .enumerate()
            {
                target[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
            }
            for _ in 0..4 {
                targets.extend_from_slice(&target);
            }
        }
        let bytes = r.test_film_footprint(&source, &targets, 1);
        for (point, bytes) in points.iter().zip(bytes.chunks_exact(128)) {
            let words: Vec<u32> = bytes
                .chunks_exact(4)
                .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            let model: Vec<Vec<f32>> = (0..4)
                .map(|row| {
                    (0..4)
                        .map(|col| f32::from_bits(words[16 + col * 4 + row]))
                        .collect()
                })
                .collect();
            rays.push(serde_json::json!({"pose":pose,"pixel":point,"origin_bits":words[..3],"direction_bits":words[3..6],"hit_t":f32::from_bits(words[6]),"front_face":words[7]!=0,"record":words[8..12],"adjacency_record":words[12..16],"model":model}));
        }
    }
    fs::write(
        out.join("difference-rays.json"),
        serde_json::to_vec_pretty(&rays).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "owned GPU lease; one loaded quarry, paired bounded retry screen"]
fn gpu_backface_retry_screen() {
    let started = Instant::now();
    let m = manifest();
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let baseline = primary_ray_gpu::shader_sources(SOURCE).unwrap();
    let candidate = baseline.each_ref().map(|s| retry_source(s));
    super::receiver_fold_tests::gpu_receiver_fold();
    let folded =
        super::receiver_fold_tests::traced_fold_with(baseline.clone(), &out.join("fold-adjacency"));
    let retry_fold =
        super::receiver_fold_tests::traced_fold_with(candidate.clone(), &out.join("fold-retry"));
    let fold_identical = folded == retry_fold;
    let _validation = validation::begin(true);
    let mut r = renderer(true);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    let upload = Instant::now();
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    let load_seconds = upload.elapsed().as_secs_f64();
    let color = target(&r, 1920, 1080);
    let views = t1::t1_cameras(&m);
    for (name, sources) in [("adjacency", &baseline), ("retry", &candidate)] {
        let dir = out.join(name);
        fs::create_dir_all(&dir).unwrap();
        let shading = primary_ray_gpu::shading_sources(SOURCE, STANDARD).unwrap();
        for (n, s) in ["kernels-hard", "kernels-soft", "traced-hard", "traced-soft"]
            .into_iter()
            .zip(sources.iter().chain(shading.iter()))
        {
            fs::write(dir.join(format!("{n}.wgsl")), s).unwrap();
        }
    }
    let mut retry = r
        .primary
        .as_ref()
        .unwrap()
        .compile(&r.device, &r.ledger, &candidate);
    let footprint = t1::footprint_source();
    let retry_footprint = retry_source(&footprint);
    let entry = footprint
        .find("@compute @workgroup_size(8,4) fn film_footprint(")
        .unwrap();
    let winding = format!(
        "{}\n{}",
        &footprint[..entry],
        include_str!("../support/receiver_winding.wgsl")
    );
    let mut micro = vec![];
    let mut winding_rows = vec![];
    let mut witness_rows = vec![];
    let witness_rays: serde_json::Value =
        serde_json::from_slice(&fs::read(m["witness_rays"].as_str().unwrap()).unwrap()).unwrap();
    for (pose, camera) in views.iter().enumerate() {
        let (surface, _) = t1::centre_surface(&mut r, &mut asset, &color, camera);
        let targets = micro_targets(&m, pose, &surface, camera);
        if pose == 0 {
            for (name, source) in [("adjacency", &footprint), ("retry", &retry_footprint)] {
                let source = witness_source(source, &witness_rays);
                validate_wgsl(&source);
                fs::write(out.join(format!("witness-{name}.wgsl")), &source).unwrap();
                let bytes = r.test_film_footprint(&source, &targets, 1);
                let words: Vec<Vec<u32>> = bytes
                    .chunks_exact(32)
                    .map(|row| {
                        row.chunks_exact(4)
                            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
                            .collect()
                    })
                    .collect();
                witness_rows.push(serde_json::json!({"variant":name,"records":words}));
            }
        }
        let mut outputs = vec![];
        for (name, source) in [("adjacency", &footprint), ("retry", &retry_footprint)] {
            let bytes = r.test_film_footprint(source, &targets, 1024);
            let rows: Vec<_> = bytes
                .chunks_exact(32)
                .map(|row| {
                    row.chunks_exact(4)
                        .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
                        .collect::<Vec<_>>()
                })
                .collect();
            micro.push(
                serde_json::json!({"pose":pose,"variant":name,"rows":rows,"sha256":digest(&bytes)}),
            );
            outputs.push(bytes);
        }
        let bytes = r.test_film_footprint(&winding, &targets[..80], 2048);
        let words: Vec<_> = bytes[..32]
            .chunks_exact(4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        winding_rows.push(serde_json::json!({"pose":pose,"hits":words[0],"back_faces":words[1],"back_faces_away_from_edges":words[2],"back_faces_near_edges":words[3],"near_edges":words[4],"micro_byte_identical":outputs[0]==outputs[1]}));
    }
    let soft = r.shadows.settings.clone();
    let mut hard = soft.clone();
    hard.directional_disk_angle = 0.;
    let repeats = m["repeats"].as_u64().unwrap_or(8) as usize;
    let mut identity = vec![];
    for (pose, camera) in views.iter().enumerate() {
        for mode in 0..3 {
            let mode_name = ["hard", "exact", "interactive"][mode];
            r.set_shadows(if mode == 0 { &hard } else { &soft })
                .unwrap();
            r.set_soft_shadows(if mode == 2 {
                crate::SoftShadows::Interactive
            } else {
                crate::SoftShadows::Exact
            });
            let mut expected = None;
            let mut baseline_stable = true;
            let mut candidate_stable = true;
            let mut same = true;
            let mut rows = vec![];
            let mut retry_first = None;
            for repeat in 0..repeats {
                for variant in 0..2 {
                    if variant == 1 {
                        std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut retry);
                    }
                    traced_frame(&mut r, &color, camera, &mut asset, true);
                    let actual = image_buffers(&mut r, mode != 0);
                    let hashes: Vec<_> = actual.iter().map(|b| digest(b)).collect();
                    if variant == 0 {
                        if let Some(e) = &expected {
                            baseline_stable &= &actual == e;
                        } else {
                            expected = Some(actual.clone());
                        }
                    } else {
                        same &= actual == *expected.as_ref().unwrap();
                        if let Some(e) = &retry_first {
                            candidate_stable &= &actual == e;
                        } else {
                            retry_first = Some(actual.clone());
                        }
                        if repeat == 0 {
                            let differences:Vec<_>=actual.iter().zip(expected.as_ref().unwrap()).enumerate().map(|(i,(a,b))| {
                                let pixels=changed_pixels(a,b,[16,8,16,16,4][i]);
                                serde_json::json!({"buffer":(["hit","hdr","world","normal","visibility"][i]),"count":pixels.len(),"first_pixels":pixels.iter().take(128).map(|p|[p%1920,p/1920]).collect::<Vec<_>>()})
                            }).collect();
                            fs::write(
                                out.join(format!("diff-{pose}-{mode_name}.json")),
                                serde_json::to_vec_pretty(&differences).unwrap(),
                            )
                            .unwrap();
                            for (name, buffers) in [
                                ("adjacency", expected.as_ref().unwrap()),
                                ("retry", &actual),
                            ] {
                                fs::write(
                                    out.join(format!("pose-{pose}-{mode_name}-{name}.rgba16f")),
                                    &buffers[1],
                                )
                                .unwrap();
                                if !same {
                                    fs::write(
                                        out.join(format!("pose-{pose}-{mode_name}-{name}.hits")),
                                        &buffers[0],
                                    )
                                    .unwrap();
                                }
                            }
                        }
                    }
                    rows.push(
                        serde_json::json!({"repeat":repeat,"variant":variant,"hashes":hashes}),
                    );
                    if variant == 1 {
                        std::mem::swap(r.pipeline.primary.as_mut().unwrap(), &mut retry);
                    }
                }
            }
            identity.push(serde_json::json!({"pose":pose,"mode":mode_name,"identity":same,"baseline_stable":baseline_stable,"candidate_stable":candidate_stable,"samples":rows}));
        }
    }
    let mut timing = m.clone();
    timing["output"] = serde_json::json!(out.join("timing"));
    timing["candidate_modules"] = serde_json::json!(out.join("retry"));
    timing["frozen_modules"] = serde_json::json!(out.join("adjacency"));
    timing["warmup_pairs"] = serde_json::json!(2);
    timing["pairs"] = serde_json::json!(8);
    super::receiver_paired_with(&timing, &mut r, &mut asset, &cameras, &color);
    fs::write(out.join("screen.json"),serde_json::to_vec_pretty(&serde_json::json!({"fold_identical":fold_identical,"quarry_loads":1,"load_seconds":load_seconds,"micro":micro,"winding":winding_rows,"witnesses":witness_rows,"identity":identity,"seconds":started.elapsed().as_secs_f64()})).unwrap()).unwrap();
}
