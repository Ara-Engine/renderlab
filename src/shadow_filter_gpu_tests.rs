//! RT1d static quarry pair, one upload, current visibility and complete HDR captures.
use super::*;
use sha2::{Digest, Sha256};

#[test]
#[ignore = "RT1d owned GPU lease, no game; both quarry poses, one asset upload"]
fn gpu_rt1d_quarry_pair() {
    let m = manifest();
    let _validation = validation::begin(true);
    let out = PathBuf::from(m["output"].as_str().unwrap());
    fs::create_dir_all(&out).unwrap();
    let mut r = renderer(true);
    let (mut asset, cameras) = load_quarry(&m, &mut r);
    r.set_primary_visibility(PrimaryVisibility::Traced).unwrap();
    r.set_soft_shadows(SoftShadows::Interactive);
    let color = target(&r, 1920, 1080);
    let visibility = centre_shading(&r, &crate::program::standard_returning(VISIBILITY), true);
    let mut rows = vec![];
    for pose in 0..2 {
        let camera = cameras.frames[pose].camera().unwrap();
        let mut hit_hash = None;
        for (name, enabled) in [("previous", false), ("filtered", true)] {
            r.set_interactive_shadow_filter(enabled).unwrap();
            let mut hashes = vec![];
            for repeat in 0..8 {
                traced_frame(&mut r, &color, &camera, &mut asset, true);
                let hdr = hdr_rect(&r, [0, 0, 1920, 1080]);
                let hash = format!("{:x}", Sha256::digest(&hdr));
                if repeat == 0 {
                    fs::write(out.join(format!("p{pose}-{name}.rgba16f")), hdr).unwrap();
                } else {
                    assert_eq!(hashes[0], hash, "RT1d repeat {repeat}, {name}");
                }
                hashes.push(hash);
            }
            let hit = crop_hit(&r, [0, 0, 1920, 1080]);
            let hash = format!("{:x}", Sha256::digest(&hit));
            if let Some(previous) = &hit_hash {
                assert_eq!(previous, &hash, "filter changed primary identity");
            }
            hit_hash = Some(hash.clone());
            fs::write(out.join(format!("p{pose}-{name}.hit.u32")), hit).unwrap();
            let mut e = r.device.create_command_encoder(&Default::default());
            r.encode_primary_shade(
                &mut e,
                &visibility[1],
                primary_ray_gpu::SHADE_TILE,
                primary_ray_gpu::Shading::Soft,
                None,
            );
            r.queue.submit([e.finish()]);
            fs::write(
                out.join(format!("p{pose}-{name}-visibility.rgba16f")),
                hdr_rect(&r, [0, 0, 1920, 1080]),
            )
            .unwrap();
            rows.push(
                serde_json::json!({"pose":pose,"name":name,"hdr_hashes":hashes,"hit_hash":hash}),
            );
        }
    }
    fs::write(
        out.join("determinism.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    r.test_drain("RT1d quarry pair");
}
