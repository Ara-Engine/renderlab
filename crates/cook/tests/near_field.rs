//! M7 G1c near-field cook: masks follow occluders under rotated, scaled placements; results
//! are deterministic across thread counts; the cache reuses, repairs and separates settings.
use renderlab_asset::{
    self as asset, Asset, Material, Mesh, Placement, Primitive, Vertex,
    near_field::{self as format, Settings},
    virtual_mesh::{self, VirtualLimits},
};
use renderlab_cook::{cluster_bvh_cache, near_field, virtual_cache, virtual_cook};
use sha2::{Digest, Sha256};

fn vertex(position: [f32; 3], normal: [f32; 3]) -> Vertex {
    Vertex {
        position,
        normal,
        uv: [0.; 2],
        color: [1.; 4],
    }
}

/// A floor, authored in the object XY plane facing +Z and placed as the quarry's scans are
/// (-90 degrees about X, uniform scale 2), so world [-2, 2]^2 at y = 0 facing +Y; and a
/// closed box slab over its middle, x and z in [-1, 1], y in [0.2, 0.3], placed by a
/// translation.
fn scene() -> Asset {
    let n = 16u32;
    let mut floor = Primitive {
        tangents: None,
        vertices: vec![],
        indices: vec![],
        material: 0,
    };
    for j in 0..=n {
        for i in 0..=n {
            let p = [
                i as f32 / n as f32 * 2. - 1.,
                j as f32 / n as f32 * 2. - 1.,
                0.,
            ];
            floor.vertices.push(vertex(p, [0., 0., 1.]));
        }
    }
    for j in 0..n {
        for i in 0..n {
            let a = j * (n + 1) + i;
            let (b, c, d) = (a + 1, a + n + 1, a + n + 2);
            floor.indices.extend([a, b, d, a, d, c]);
        }
    }
    let mut slab = Primitive {
        tangents: None,
        vertices: vec![],
        indices: vec![],
        material: 0,
    };
    let half = [1f32, 0.05, 1.];
    for axis in 0..3 {
        for sign in [-1f32, 1.] {
            let (t1, t2) = ((axis + 1) % 3, (axis + 2) % 3);
            let base = slab.vertices.len() as u32;
            let mut normal = [0.; 3];
            normal[axis] = sign;
            for (u, v) in [(-1f32, -1f32), (1., -1.), (1., 1.), (-1., 1.)] {
                let mut p = [0.; 3];
                p[axis] = sign * half[axis];
                p[t1] = u * half[t1];
                p[t2] = v * half[t2];
                slab.vertices.push(vertex(p, normal));
            }
            if sign > 0. {
                slab.indices
                    .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            } else {
                slab.indices
                    .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
            }
        }
    }
    let floor_transform = [
        [2., 0., 0., 0.],
        [0., 0., -2., 0.],
        [0., 2., 0., 0.],
        [0., 0., 0., 1.],
    ];
    let slab_transform = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0.25, 0., 1.],
    ];
    Asset {
        meshes: vec![
            Mesh {
                primitives: vec![floor],
            },
            Mesh {
                primitives: vec![slab],
            },
        ],
        placements: vec![
            Placement {
                mesh: 0,
                transform: floor_transform,
            },
            Placement {
                mesh: 1,
                transform: slab_transform,
            },
        ],
        materials: vec![Material::default()],
        textures: vec![],
    }
}

fn settings() -> Settings {
    Settings {
        lod_error: 0.,
        near: 0.25,
        reach: 0.5,
        skip: 0.0,
        bins: 16,
    }
}

fn prepare(c: &virtual_cook::Cooked, threads: usize) -> format::NearField {
    near_field::prepare(
        &c.package,
        settings(),
        VirtualLimits::default(),
        threads,
        |p| {
            Ok(virtual_mesh::decode_page(
                &c.package.pages[p],
                &c.pages[p],
                Sha256::digest(&c.pages[p]).into(),
            )?)
        },
    )
    .unwrap()
    .0
}

/// The front and back masks of the floor vertex nearest world (x, 0, z).
fn floor_masks(f: &format::NearField, x: f32, z: f32) -> (Vec<u32>, Vec<u32>) {
    let words = f.settings.words();
    let positions = &f.meshes[0].positions;
    // Object (u, v, 0) maps to world (2u, 0, -2v).
    let (u, v) = (x / 2., -z / 2.);
    let i = (0..positions.len())
        .min_by(|&a, &b| {
            let d = |k: usize| (positions[k][0] - u).powi(2) + (positions[k][1] - v).powi(2);
            d(a).total_cmp(&d(b))
        })
        .unwrap();
    let m = &f.placements[0].masks;
    (
        m[i * 2 * words..(i * 2 + 1) * words].to_vec(),
        m[(i * 2 + 1) * words..(i * 2 + 2) * words].to_vec(),
    )
}
fn blocked(mask: &[u32], d: [f32; 3]) -> bool {
    let b = format::bin(d, 16);
    mask[(b / 32) as usize] & (1 << (b % 32)) != 0
}

#[test]
fn masks_follow_occluders_under_rotated_scaled_placements() {
    let c = virtual_cook::prepare(
        &scene(),
        virtual_cook::Settings::default(),
        VirtualLimits::default(),
    )
    .unwrap();
    let f = prepare(&c, 3);
    f.validate(format::Limits::default()).unwrap();
    assert_eq!(f.meshes.len(), 2);
    assert_eq!(f.placements.len(), 2);
    let (up, down) = ([0., 1., 0.], [0., -1., 0.]);
    // Under the slab, 0.2 m below it: up is blocked on the front side.
    let (front, back) = floor_masks(&f, 0., 0.);
    assert!(blocked(&front, up) && blocked(&front, down));
    // Behind the floor, up passes through the floor itself; down is open.
    assert!(blocked(&back, up) && !blocked(&back, down));
    // In the open, beyond the slab's reach: up is open.
    let (front, _) = floor_masks(&f, 1.75, 1.75);
    assert!(!blocked(&front, up) && blocked(&front, down));
    // A low direction (about 33 degrees up) toward the slab from 0.25 m outside its edge
    // meets its side or underside within reach, whichever bin centre it lands on; the same
    // direction away from it is open.
    let (front, _) = floor_masks(&f, 1.25, 0.);
    let toward = [-0.83, 0.55, 0.];
    let away = [0.83, 0.55, 0.];
    assert!(blocked(&front, toward) && !blocked(&front, away));
}

#[test]
fn cook_is_deterministic_across_threads_and_cached() {
    let a = scene();
    let c = virtual_cook::prepare(
        &a,
        virtual_cook::Settings::default(),
        VirtualLimits::default(),
    )
    .unwrap();
    let one = format::encode(&prepare(&c, 1), format::Limits::default()).unwrap();
    for threads in [2, 5] {
        assert_eq!(
            format::encode(&prepare(&c, threads), format::Limits::default()).unwrap(),
            one
        );
    }
    let d = std::env::temp_dir().join(format!("renderlab-g1c-near-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let limits = VirtualLimits::default();
    let source = d.join("source.rla");
    std::fs::write(&source, asset::encode(&a, limits.prepared).unwrap()).unwrap();
    let raster = d.join("raster.manifest");
    virtual_cache::cook_file(
        &source,
        &d,
        &raster,
        virtual_cook::Settings::default(),
        limits,
    )
    .unwrap();
    let input = cluster_bvh_cache::load_geometry(&d, &raster, limits).unwrap();
    let manifest = d.join("near.manifest");
    let cold = near_field::cook(&input, &d, &d, &manifest, settings(), limits, 2).unwrap();
    assert!(!cold.cache_hit);
    assert_eq!(cold.field.source, input.digest);
    let warm = near_field::cook(&input, &d, &d, &manifest, settings(), limits, 2).unwrap();
    assert!(warm.cache_hit);
    assert_eq!(cold.digest, warm.digest);
    assert_eq!(
        near_field::load(&d, &manifest, format::Limits::default()).unwrap(),
        cold.field
    );
    // A corrupted package is rebuilt to the same digest.
    let package = d.join(format!(
        "near-field-v{}/rla{}-rvg{}/packages/{}",
        format::VERSION,
        input.rla_version,
        input.rvg_version,
        cold.digest
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>()
    ));
    std::fs::write(package, b"broken").unwrap();
    let repaired = near_field::cook(&input, &d, &d, &manifest, settings(), limits, 2).unwrap();
    assert!(!repaired.cache_hit);
    assert_eq!(cold.digest, repaired.digest);
    // Other settings get their own key; invalid ones publish nothing.
    let changed = near_field::cook(
        &input,
        &d,
        &d,
        &manifest,
        Settings {
            bins: 8,
            ..settings()
        },
        limits,
        2,
    )
    .unwrap();
    assert_ne!(cold.key, changed.key);
    let published = std::fs::read(&manifest).unwrap();
    assert!(
        near_field::cook(
            &input,
            &d,
            &d,
            &manifest,
            Settings {
                reach: 0.1,
                ..settings()
            },
            limits,
            2
        )
        .is_err()
    );
    assert_eq!(published, std::fs::read(&manifest).unwrap());
    std::fs::remove_dir_all(d).unwrap();
}
