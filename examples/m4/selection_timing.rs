use super::*;

#[cfg(not(feature = "selection-oracle"))]
pub fn run(_: &Options, _: &cameras::Cameras) -> Result<(), Box<dyn std::error::Error>> {
    Err("CPU paired harness requires --features selection-oracle (without selection-shadow)".into())
}

#[cfg(feature = "selection-oracle")]
pub fn run(o: &Options, cameras: &cameras::Cameras) -> Result<(), Box<dyn std::error::Error>> {
    use asset::virtual_mesh::*;
    use sha2::{Digest, Sha256};
    if cfg!(feature = "selection-shadow") {
        return Err("never time with selection-shadow".into());
    }
    let asset = VirtualAsset::open(&o.cache, &o.manifest, o.stream)?;
    let mut rows = BufWriter::new(fs::File::create(o.output.join("selection.csv"))?);
    writeln!(
        rows,
        "frame,placement,availability,initial,repeat,selector,ns,visits,candidates,clusters,requested,errors,input_sha256,output_sha256"
    )?;
    let mut snapshots = BufWriter::new(fs::File::create(o.output.join("inputs.jsonl"))?);
    let mut poses: BTreeSet<_> = (o.start..(o.start + o.frames).min(cameras.frames.len()))
        .step_by(o.cpu_stride)
        .collect();
    for p in [150, 210, 269, 270, 277, 300, 330, 630] {
        if p >= o.start && p < o.start + o.frames {
            poses.insert(p);
        }
    }
    for frame in poses {
        guard()?;
        let pose = &cameras.frames[frame];
        for (slot, placement) in asset.package().placements.iter().enumerate() {
            let mesh = &asset.package().meshes[placement.mesh as usize];
            let errors = project_errors(
                mesh,
                &pose.camera()?,
                Mat4::from_cols_array_2d(&placement.transform),
                pose.height,
            )?;
            let b = SelectionBudget {
                geometry_bytes: o.stream.geometry_bytes - PAGE_BYTES,
                max_candidates: o.stream.max_candidates,
                max_clusters: o.stream.max_clusters,
                max_scratch_bytes: METADATA_LIMIT,
            };
            let root_errors: Vec<_> = mesh
                .groups
                .iter()
                .map(|g| {
                    if g.simplified.error == f32::MAX {
                        f64::INFINITY
                    } else {
                        0.
                    }
                })
                .collect();
            let roots = select(
                mesh,
                &root_errors,
                1.,
                &vec![true; asset.package().pages.len()],
                b,
            )?
            .cut;
            for all in [true, false] {
                let mut available = vec![all; asset.package().pages.len()];
                for p in &roots.pages {
                    available[p.index()] = true;
                }
                for from_cut in [false, true] {
                    let initial = select(
                        mesh,
                        &errors,
                        o.stream.error_pixels,
                        &available,
                        SelectionBudget {
                            max_candidates: 64,
                            ..b
                        },
                    )?
                    .cut;
                    let protected = &roots.pages;
                    let input = serde_json::json!({"frame": frame,"placement": slot,"errors_bits": errors.iter().map(|e| e.to_bits()).collect::<Vec<_>>(),"available": available,"initial": from_cut.then_some(initial.active_groups.iter().map(|g|g.0).collect::<Vec<_>>()),"protected": protected.iter().map(|p|p.0).collect::<Vec<_>>(),"geometry_bytes": b.geometry_bytes,"candidates":b.max_candidates,"clusters":b.max_clusters,"threshold_bits":o.stream.error_pixels.to_bits()});
                    let encoded = serde_json::to_vec(&input)?;
                    let identity = format!("{:x}", Sha256::digest(&encoded));
                    snapshots.write_all(&encoded)?;
                    snapshots.write_all(b"\n")?;
                    let call = |old| {
                        if old {
                            if from_cut {
                                selection_oracle::select_from_cut(
                                    mesh,
                                    &errors,
                                    o.stream.error_pixels,
                                    &available,
                                    b,
                                    protected,
                                    &initial,
                                )
                            } else {
                                selection_oracle::select_with_protected(
                                    mesh,
                                    &errors,
                                    o.stream.error_pixels,
                                    &available,
                                    b,
                                    protected,
                                )
                            }
                        } else if from_cut {
                            select_from_cut(
                                mesh,
                                &errors,
                                o.stream.error_pixels,
                                &available,
                                b,
                                protected,
                                &initial,
                            )
                        } else {
                            select_with_protected(
                                mesh,
                                &errors,
                                o.stream.error_pixels,
                                &available,
                                b,
                                protected,
                            )
                        }
                    };
                    // Warm both sides and verify before timing. All input creation/projection/hash work is outside.
                    let a = call(false)?;
                    let z = call(true)?;
                    assert_eq!(
                        (&a.cut, &a.demand, &a.debt, a.candidates),
                        (&z.cut, &z.demand, &z.debt, z.candidates)
                    );
                    let output = format!(
                        "{:x}",
                        Sha256::digest(
                            format!("{:?}{:?}{:?}{}", a.cut, a.demand, a.debt, a.candidates)
                                .as_bytes()
                        )
                    );
                    for repeat in 0..o.repeats {
                        for old in if repeat % 2 == 0 {
                            [true, false]
                        } else {
                            [false, true]
                        } {
                            let start = Instant::now();
                            let result = std::hint::black_box(call(old));
                            let ns = start.elapsed().as_nanos();
                            let s = result?;
                            assert_eq!(
                                (&s.cut, &s.demand, &s.debt, s.candidates),
                                (&a.cut, &a.demand, &a.debt, a.candidates)
                            );
                            writeln!(
                                rows,
                                "{frame},{slot},{},{from_cut},{repeat},{},{ns},{},{},{},{},0,{identity},{output}",
                                if all { "all" } else { "roots" },
                                if old { "oracle" } else { "incremental" },
                                s.cluster_visits,
                                s.candidates,
                                s.cut.clusters.len(),
                                s.demand.requested_pages.len()
                            )?;
                        }
                    }
                }
            }
        }
    }
    rows.flush()?;
    snapshots.flush()?;
    Ok(())
}
