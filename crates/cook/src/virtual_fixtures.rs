//! Self-authored, bounded M4 inputs. Registration precedes selection experiments.
use crate::{Result, ensure, hash_hex};
use renderlab_asset::{Asset, Material, Mesh, Placement, Primitive, Vertex};
use serde_json::json;
use std::{fs, path::Path};

const IDENTITY: [[f32; 4]; 4] = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

fn asset(primitives: Vec<Primitive>, materials: Vec<Material>) -> Asset {
    Asset {
        meshes: vec![Mesh { primitives }],
        placements: vec![Placement {
            mesh: 0,
            transform: IDENTITY,
        }],
        materials,
        textures: vec![],
    }
}

/// Six independently UV-mapped faces, welded by exact position bits for topology proofs.
/// Integer cube coordinates are normalized in canonical XYZ order on every face.
pub fn closed_sphere(n: u32) -> Result<Asset> {
    ensure((2..=384).contains(&n), "sphere subdivisions outside 2..384")?;
    let mut p = Primitive {
        adjacency: None,
        tangents: None,
        vertices: vec![],
        indices: vec![],
        material: 0,
    };
    for axis in 0..3 {
        for sign in [-1i32, 1] {
            let base = p.vertices.len() as u32;
            for y in 0..=n {
                for x in 0..=n {
                    let mut q = [0i32; 3];
                    q[axis] = sign * n as i32;
                    q[(axis + 1) % 3] = 2 * x as i32 - n as i32;
                    q[(axis + 2) % 3] = 2 * y as i32 - n as i32;
                    let length = q.map(|v| f64::from(v).powi(2)).iter().sum::<f64>().sqrt();
                    let position = q.map(|v| {
                        if v == 0 {
                            0.
                        } else {
                            (f64::from(v) / length) as f32
                        }
                    });
                    p.vertices.push(Vertex {
                        position,
                        normal: position,
                        uv: [x as f32 / n as f32, y as f32 / n as f32],
                        color: [1.; 4],
                    });
                }
            }
            for y in 0..n {
                for x in 0..n {
                    let a = base + y * (n + 1) + x;
                    let b = a + 1;
                    let c = a + n + 1;
                    let d = c + 1;
                    if sign > 0 {
                        p.indices.extend([a, b, d, a, d, c]);
                    } else {
                        p.indices.extend([a, d, b, a, c, d]);
                    }
                }
            }
        }
    }
    Ok(asset(vec![p], vec![Material::default()]))
}

/// Curved two-material sheet plus separate thin triangular fins. Open sheet/fins are intentional.
pub fn thin_sheet(n: u32) -> Result<Asset> {
    ensure(
        (4..=768).contains(&n) && n.is_multiple_of(2),
        "sheet needs even subdivisions 4..768",
    )?;
    let mut primitives = vec![];
    for side in 0..2 {
        let mut p = Primitive {
            adjacency: None,
            tangents: None,
            vertices: vec![],
            indices: vec![],
            material: side,
        };
        for y in 0..=n {
            for x in side * n / 2..=(side + 1) * n / 2 {
                let u = f64::from(x) / f64::from(n);
                let v = f64::from(y) / f64::from(n);
                let (sx, cx) = (u * 6.).sin_cos();
                let (sy, cy) = (v * 5.).sin_cos();
                let normal = [-0.3 * cx * cy, 1., 0.25 * sx * sy];
                let length = normal.iter().map(|v| v * v).sum::<f64>().sqrt();
                p.vertices.push(Vertex {
                    position: [
                        (u * 4. - 2.) as f32,
                        (0.2 * sx * cy) as f32,
                        (v * 4. - 2.) as f32,
                    ],
                    normal: normal.map(|v| (v / length) as f32),
                    uv: [u as f32, v as f32],
                    color: [1.; 4],
                });
            }
        }
        let row = n / 2 + 1;
        for y in 0..n {
            for x in 0..n / 2 {
                let a = y * row + x;
                let b = a + 1;
                let c = a + row;
                let d = c + 1;
                p.indices.extend([a, d, b, a, c, d]);
            }
        }
        primitives.push(p);
    }
    let mut fins = Primitive {
        adjacency: None,
        tangents: None,
        vertices: vec![],
        indices: vec![],
        material: 1,
    };
    for i in 0..48 {
        let z = i as f32 / 47. * 3.8 - 1.9;
        let a = fins.vertices.len() as u32;
        for position in [
            [-1.95, 0.25, z],
            [-1.945, 0.25, z + 0.025],
            [-1.94, 0.9, z + 0.0125],
        ] {
            fins.vertices.push(Vertex {
                position,
                normal: [1., 0., 0.],
                uv: [0., 0.],
                color: [1.; 4],
            });
        }
        fins.indices.extend([a, a + 1, a + 2]);
    }
    primitives.push(fins);
    Ok(asset(
        primitives,
        vec![
            Material {
                base_color: [0.08, 0.5, 0.9, 1.],
                double_sided: true,
                ..Material::default()
            },
            Material {
                base_color: [0.95, 0.2, 0.03, 1.],
                double_sided: true,
                ..Material::default()
            },
        ],
    ))
}

pub fn cameras() -> serde_json::Value {
    let mut frames = vec![];
    let mut add = |phase: &str, eye: [f64; 3], target: [f64; 3]| {
        frames.push(json!({"frame":frames.len(),"phase":phase,"eye":eye,"target":target,"up":[0,1,0],"width":1920,"height":1080,"fov_y_radians":1.0471975511965976,"near":0.05,"far":100.0}))
    };
    for _ in 0..60 {
        add("overview", [4., 3., 5.], [0., 0., 0.]);
    }
    for i in 0..90 {
        let t = i as f64 / 89.;
        add(
            "approach",
            [4. - 3.2 * t, 3. - 2.4 * t, 5. - 3.5 * t],
            [0., 0., 0.],
        );
    }
    for i in 0..120 {
        let a = i as f64 / 119. * std::f64::consts::TAU;
        add("sweep", [2.2 * a.cos(), 1.1, 2.2 * a.sin()], [0., 0., 0.]);
    }
    add("abrupt-cut", [-0.4, 0.3, -1.2], [0., 0., 0.]);
    for i in 0..60 {
        let t = i as f64 / 59.;
        add(
            "return",
            [-0.4 + 4.4 * t, 0.3 + 2.7 * t, -1.2 + 6.2 * t],
            [0., 0., 0.],
        );
    }
    for _ in 0..300 {
        add("settled", [4., 3., 5.], [0., 0., 0.]);
    }
    json!({"schema":1,"frames":frames})
}

pub fn register(directory: &Path) -> Result<()> {
    fs::create_dir_all(directory)?;
    ensure(
        !directory.join("registration.json").exists(),
        "registration exists: preserve it and choose a new directory",
    )?;
    let cameras = serde_json::to_vec_pretty(&cameras()).unwrap();
    fs::write(directory.join("cameras.json"), &cameras)?;
    let mut records = vec![];
    for (name, settings, a) in [
        (
            "closed-sphere",
            json!({"subdivisions":288,"faces":6}),
            closed_sphere(288)?,
        ),
        (
            "material-thin",
            json!({"subdivisions":704,"materials":2,"fins":48}),
            thin_sheet(704)?,
        ),
    ] {
        let limits = renderlab_asset::Limits::default();
        let counts = a.validate(limits)?;
        let bytes = renderlab_asset::encode(&a, limits)?;
        let geometry_bytes = counts.vertices * 48 + counts.indices * 4;
        ensure(
            geometry_bytes > 4 * (8 << 20),
            "fixture must exceed four times initial normal pool",
        )?;
        fs::write(directory.join(format!("{name}.rla")), &bytes)?;
        let materials = renderlab_asset::encode(
            &Asset {
                materials: a.materials.clone(),
                textures: a.textures.clone(),
                ..Asset::default()
            },
            limits,
        )?;
        records.push(json!({"name":name,"generator_settings":settings,"prepared_file":format!("{name}.rla"),"prepared_sha256":hash_hex(&bytes),"source_sha256":hash_hex(&bytes),"material_texture_sha256":hash_hex(&materials),"vertices":counts.vertices,"indices":counts.indices,"full_detail_geometry_bytes":geometry_bytes,"declared_normal_pool_floor_bytes":8<<20}));
    }
    let command = |args: &[&str]| {
        std::process::Command::new(args[0])
            .args(&args[1..])
            .output()
            .ok()
            .map(|v| String::from_utf8_lossy(&v.stdout).trim().to_owned())
    };
    let executable = std::env::current_exe()?;
    let registration = json!({"schema":1,"stage":"fixture registration (preserve initial pre-selector record)","source_kind":"direct prepared procedural geometry; GLB subset unchanged","generator_source_file":"crates/cook/src/virtual_fixtures.rs","generator_source_sha256":hash_hex(include_bytes!("virtual_fixtures.rs")),"cargo_lock_sha256":hash_hex(include_bytes!("../../../Cargo.lock")),"executable_sha256":hash_hex(&fs::read(executable)?),"git_head":command(&["git","rev-parse","HEAD"]),"rustc":command(&["rustc","--version"]),"camera_json_sha256":hash_hex(&cameras),"fixtures":records,"density_reason":"36MB-class full-detail geometry exceeds 4*8MiB while retaining default 2M vertex/6M index/256MiB caps; procedural prepared input avoids changing the 64MiB GLB contract","portability":"trigonometric bit identity is registered for this pinned executable/host, not promised across platforms"});
    fs::write(
        directory.join("registration.json"),
        serde_json::to_vec_pretty(&registration).unwrap(),
    )?;
    println!("{registration}");
    Ok(())
}
