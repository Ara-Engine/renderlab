//! The authored M5-A fixture. The same data drives rendering and UE matching export.
use renderlab::{Camera, Light, Lighting, Mat4, Vec3, Vertex};

pub struct Object {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub transform: Mat4,
    pub base: [f32; 4],
    pub metal: f32,
    pub rough: f32,
    pub double_sided: bool,
    pub recipe: serde_json::Value,
}
pub struct Fixture {
    pub objects: Vec<Object>,
    pub eye: Vec3,
    pub target: Vec3,
    pub lighting: Lighting,
}
impl Fixture {
    pub fn new() -> Self {
        let mut objects = vec![];
        for row in 0..2 {
            for (col, rough) in [0.1, 0.5, 1.].into_iter().enumerate() {
                let (vertices, indices) = sphere();
                objects.push(Object{name:format!("{}-rough-{rough}",if row==0 {"dielectric"} else {"metal"}),vertices,indices,transform:Mat4::from_translation(Vec3::new((col as f32-1.)*1.5,0.55,-row as f32*1.6)),base:if row==0 {[0.18,0.4,0.7,1.]}else{[0.7,0.32,0.1,1.]},metal:row as f32,rough,double_sided:false,recipe:serde_json::json!({"shape":"uv_sphere","radius":0.5,"longitude_segments":64,"latitude_segments":32})});
            }
        }
        let mut panel = |name: &str, transform: Mat4, double_sided: bool, reverse: bool| {
            let vertices = [
                [-0.5, -0.5, 0.],
                [0.5, -0.5, 0.],
                [0.5, 0.5, 0.],
                [-0.5, 0.5, 0.],
            ]
            .map(|position| Vertex {
                position,
                normal: [0., 0., 1.],
                uv: [0.; 2],
                color: [1.; 4],
            })
            .to_vec();
            objects.push(Object{name:name.into(),vertices,indices:if reverse {vec![0,2,1,0,3,2]}else{vec![0,1,2,0,2,3]},transform,base:[0.5,0.5,0.5,1.],metal:0.,rough:0.5,double_sided,recipe:serde_json::json!({"shape":"xy_quad","width":1,"height":1,"authored_normal":[0,0,1],"reverse_indices":reverse})});
        };
        panel(
            "floor",
            Mat4::from_translation(Vec3::new(0., 0., -0.6))
                * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
                * Mat4::from_scale(Vec3::new(8., 6., 1.)),
            false,
            false,
        );
        panel(
            "mirrored_nonuniform",
            Mat4::from_translation(Vec3::new(-2.8, 0.8, 0.))
                * Mat4::from_rotation_y(0.3)
                * Mat4::from_scale(Vec3::new(-0.6, 1.3, 1.)),
            false,
            false,
        );
        panel(
            "source_back_double_sided",
            Mat4::from_translation(Vec3::new(2.8, 0.8, 0.)) * Mat4::from_rotation_y(-0.3),
            true,
            true,
        );
        Self {
            objects,
            eye: Vec3::new(0., 3.0, 7.5),
            target: Vec3::new(0., 0.4, -0.7),
            lighting: Lighting {
                lights: vec![
                    Light::Directional {
                        travel: Vec3::new(-0.4, -0.8, -0.3),
                        lux: [1000., 950., 850.],
                    },
                    Light::Point {
                        position: Vec3::new(-2., 2., 1.),
                        candela: [100.; 3],
                        range: Some(6.),
                        source_radius: 0.0,
                        cast_shadows: None,
                    },
                    Light::Spot {
                        position: Vec3::new(2., 3., 2.),
                        travel: Vec3::new(0., -1., -0.5),
                        candela: [250., 300., 400.],
                        range: Some(7.),
                        inner: 0.25,
                        outer: 0.5,
                        source_radius: 0.0,
                        cast_shadows: None,
                    },
                ],
                environment: [4., 5., 8.],
                exposure_stops: -8.,
            },
        }
    }
    pub fn camera(&self, pose: usize) -> Camera {
        Camera::look_at(
            self.eye + Vec3::new(pose as f32 * 0.04, 0., 0.),
            self.target,
            Vec3::Y,
            0.78,
            0.05,
            50.,
        )
        .unwrap()
    }
    pub fn record(&self, width: u32, height: u32) -> serde_json::Value {
        let lights:Vec<_>=self.lighting.lights.iter().map(|l|match l {
            Light::Directional{travel,lux}=>serde_json::json!({"type":"directional","travel":travel.to_array(),"lux":lux}),
            Light::Point{position,candela,range, ..}=>serde_json::json!({"type":"point","position":position.to_array(),"candela":candela,"range":range}),
            Light::Spot{position,travel,candela,range,inner,outer, ..}=>serde_json::json!({"type":"spot","position":position.to_array(),"travel":travel.to_array(),"candela":candela,"range":range,"inner":inner,"outer":outer}),
        }).collect();
        serde_json::json!({"fixture":"M5-A authored v1","units":"metres","coordinates":"right_handed_Y_up_camera_forward_minus_Z","width":width,"height":height,
            "camera":{"eye":self.eye.to_array(),"target":self.target.to_array(),"up":[0,1,0],"fov_y_radians":0.78,"near":0.05,"far":50.0,"motion_eye_dx_per_pose":0.04},
            "lighting":{"lights":lights,"environment":self.lighting.environment,"exposure_stops":self.lighting.exposure_stops},
            "shading":{"dielectric_F0":0.04,"diffuse":"plain_nonmetallic_Lambert","specular":"GGX_correlated_Smith_Schlick","alpha":"max(perceptual_roughness^2,1e-4)","tone":"per_channel_Reinhard","shadows":false,"normal_maps":false,"GI":false},
            "objects":self.objects.iter().map(|o|serde_json::json!({"name":o.name,"geometry":o.recipe,"transform_column_major":o.transform.to_cols_array(),"base_color_linear":o.base,"metallic":o.metal,"perceptual_roughness":o.rough,"double_sided":o.double_sided,"vertices":o.vertices.len(),"triangles":o.indices.len()/3})).collect::<Vec<_>>()})
    }
}
fn sphere() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = vec![];
    let mut indices = vec![];
    for y in 0..=32 {
        let theta = std::f32::consts::PI * y as f32 / 32.;
        for x in 0..=64 {
            let phi = std::f32::consts::TAU * x as f32 / 64.;
            let n = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            vertices.push(Vertex {
                position: (n * 0.5).to_array(),
                normal: n.to_array(),
                uv: [0.; 2],
                color: [1.; 4],
            });
        }
    }
    for y in 0..32 {
        for x in 0..64 {
            let a = y * 65 + x;
            let b = a + 65;
            indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    (vertices, indices)
}
