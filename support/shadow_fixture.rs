//! Owned opaque shadow fixture. No external assets or cooker are needed.
use renderlab::{Camera, Light, Lighting, Mat4, Vec3, Vertex};

pub struct Object {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub transform: Mat4,
    pub base: [f32; 4],
    pub metal: f32,
    pub rough: f32,
    pub double_sided: bool,
}
pub struct Fixture {
    pub objects: Vec<Object>,
    pub lighting: Lighting,
}
impl Fixture {
    pub fn new() -> Self {
        let mut objects = vec![box_object(
            Vec3::new(0., -0.15, 0.),
            Vec3::new(14., 0.3, 14.),
            [0.55, 0.55, 0.55, 1.],
        )];
        objects.push(box_object(
            Vec3::new(0., 1., 0.),
            Vec3::new(1.2, 2., 1.2),
            [0.65, 0.22, 0.12, 1.],
        ));
        objects.push(box_object(
            Vec3::new(-2.4, 0.75, 1.),
            Vec3::new(-1.3, 1.5, 1.),
            [0.12, 0.35, 0.65, 1.],
        ));
        objects.push(box_object(
            Vec3::new(2.2, 1.5, -1.),
            Vec3::new(0.035, 3., 2.),
            [0.6, 0.58, 0.2, 1.],
        ));
        objects.push(box_object(
            Vec3::new(-1., 2.8, -3.),
            Vec3::new(3., 0.05, 0.5),
            [0.2, 0.6, 0.3, 1.],
        ));
        objects[4].double_sided = true;
        let lighting = Lighting {
            lights: vec![
                Light::Directional {
                    travel: Vec3::new(-0.55, -1., -0.25),
                    lux: [1000.; 3],
                },
                Light::Point {
                    position: Vec3::new(3., 3., 2.),
                    candela: [3500., 2000., 1000.],
                    range: Some(15.),
                    source_radius: 0.0,
                },
                Light::Spot {
                    position: Vec3::new(-3., 4.5, 3.),
                    travel: Vec3::new(0.4, -1., -0.6),
                    candela: [1800., 2500., 4500.],
                    range: Some(18.),
                    inner: 0.3,
                    outer: 0.65,
                    source_radius: 0.0,
                },
            ],
            environment: [4.; 3],
            exposure_stops: -9.,
        };
        Self { objects, lighting }
    }
    pub fn camera(&self, pose: usize) -> Camera {
        let t = pose as f32 * 0.035;
        Camera::look_at(
            Vec3::new(7.5 + t, 5.5, 9.5 - t),
            Vec3::new(0., 0.8, 0.),
            Vec3::Y,
            50_f32.to_radians(),
            0.1,
            100.,
        )
        .unwrap()
    }
    pub fn record(&self, width: u32, height: u32) -> serde_json::Value {
        serde_json::json!({"scene":"owned hard-shadow fixture", "width":width,"height":height,
            "lighting":format!("{:?}",self.lighting),
            "objects":self.objects.iter().map(|o| serde_json::json!({"transform":o.transform.to_cols_array(),"base":o.base,"metal":o.metal,"rough":o.rough,"double_sided":o.double_sided,
                "positions":o.vertices.iter().map(|v|v.position).collect::<Vec<_>>(),"normals":o.vertices.iter().map(|v|v.normal).collect::<Vec<_>>(),"indices":o.indices})).collect::<Vec<_>>()})
    }
}
fn box_object(position: Vec3, scale: Vec3, base: [f32; 4]) -> Object {
    let mut vertices = vec![];
    let mut indices = vec![];
    for (normal, u, v) in [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (-Vec3::X, Vec3::Y, -Vec3::Z),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (-Vec3::Y, Vec3::Z, -Vec3::X),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (-Vec3::Z, Vec3::X, -Vec3::Y),
    ] {
        let first = vertices.len() as u32;
        for [x, y] in [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]] {
            vertices.push(Vertex {
                position: ((normal + u * x + v * y) * 0.5).to_array(),
                normal: normal.to_array(),
                uv: [(x + 1.) * 0.5, (y + 1.) * 0.5],
                color: [1.; 4],
            });
        }
        indices.extend([0, 1, 2, 0, 2, 3].map(|i| i + first));
    }
    Object {
        vertices,
        indices,
        transform: Mat4::from_translation(position) * Mat4::from_scale(scale),
        base,
        metal: 0.,
        rough: 0.8,
        double_sided: false,
    }
}

pub fn moved_lighting(
    lighting: &Lighting,
    pose: usize,
    motion: &str,
) -> Result<Lighting, &'static str> {
    let mut value = lighting.clone();
    let t = pose as f32 * 0.025;
    let mut moved = false;
    for light in &mut value.lights {
        match (motion, light) {
            ("sun", Light::Directional { travel, .. }) => {
                travel.x += t;
                moved = true;
            }
            ("point", Light::Point { position, .. }) => {
                position.x += t;
                moved = true;
            }
            ("spot", Light::Spot { travel, .. }) => {
                travel.x += t;
                moved = true;
            }
            _ => {}
        }
    }
    if ["sun", "point", "spot"].contains(&motion) && !moved {
        return Err("requested moving light type is absent from the lighting set");
    }
    Ok(value)
}
