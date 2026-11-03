//! Self-authored normal-map fixture and preparation recipe. No third-party artwork.
use crate::{
    Error, Result,
    normal_map::{self, PreparedNormalMap},
    tangent::{self, PreparedTangentMesh, TangentSource},
};
use image::{ExtendedColorType, ImageEncoder};
use renderlab_asset::Vertex;
use serde_json::{Value, json};
use std::{fs, path::Path};

pub struct NormalFixture {
    pub glb: Vec<u8>,
    pub authored: PreparedTangentMesh,
    pub generated: PreparedTangentMesh,
    pub normal_map: PreparedNormalMap,
}

struct Buffer {
    bytes: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}
impl Buffer {
    fn view(&mut self, bytes: &[u8]) -> usize {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
        let i = self.views.len();
        self.views
            .push(json!({"buffer":0,"byteOffset":self.bytes.len(),"byteLength":bytes.len()}));
        self.bytes.extend_from_slice(bytes);
        i
    }
    fn accessor(&mut self, bytes: &[u8], count: usize, kind: &str, component: u32) -> usize {
        let view = self.view(bytes);
        let i = self.accessors.len();
        self.accessors
            .push(json!({"bufferView":view,"componentType":component,"count":count,"type":kind}));
        i
    }
    fn primitive(
        &mut self,
        vertices: &[Vertex],
        indices: &[u32],
        tangents: Option<&[[f32; 4]]>,
    ) -> Value {
        let positions: Vec<_> = vertices
            .iter()
            .flat_map(|v| v.position)
            .flat_map(f32::to_le_bytes)
            .collect();
        let normals: Vec<_> = vertices
            .iter()
            .flat_map(|v| v.normal)
            .flat_map(f32::to_le_bytes)
            .collect();
        let uv: Vec<_> = vertices
            .iter()
            .flat_map(|v| v.uv)
            .flat_map(f32::to_le_bytes)
            .collect();
        let position = self.accessor(&positions, vertices.len(), "VEC3", 5126);
        self.accessors[position]["min"] = json!([-1.0, 0.0, 0.0]);
        self.accessors[position]["max"] = json!([1.0, 1.0, 0.0]);
        let mut attributes = json!({"POSITION":position,
            "NORMAL":self.accessor(&normals,vertices.len(),"VEC3",5126),
            "TEXCOORD_0":self.accessor(&uv,vertices.len(),"VEC2",5126)});
        if let Some(tangents) = tangents {
            let bytes: Vec<_> = tangents
                .iter()
                .flatten()
                .copied()
                .flat_map(f32::to_le_bytes)
                .collect();
            attributes["TANGENT"] = json!(self.accessor(&bytes, tangents.len(), "VEC4", 5126));
        }
        let indices_bytes: Vec<_> = indices.iter().copied().flat_map(u32::to_le_bytes).collect();
        json!({"attributes":attributes,"indices":self.accessor(&indices_bytes,indices.len(),"SCALAR",5125),"material":0,"mode":4})
    }
}

fn png(width: u32, height: u32, rgba8: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(rgba8, width, height, ExtendedColorType::Rgba8)
        .map_err(|e| Error(e.to_string()))?;
    Ok(bytes)
}

/// Two mirrored-UV triangle pairs, one authored and one generated, plus a mirrored
/// nonuniform placement. The 5x3 normal swatch exercises +Y and odd mip edges.
pub fn prepare() -> Result<NormalFixture> {
    let vertices = [
        ([0.0, 0.0, 0.0], [0.0, 0.0]),
        ([1.0, 0.0, 0.0], [1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [1.0, 0.0]),
    ]
    .map(|(position, uv)| Vertex {
        position,
        uv,
        normal: [0.0, 0.0, 1.0],
        color: [1.0; 4],
    });
    let indices = [0, 1, 2, 0, 2, 3];
    let authored_vertices: Vec<_> = indices.iter().map(|&i| vertices[i as usize]).collect();
    let authored_tangents = [[1.0, 0.0, 0.0, -1.0]; 3]
        .into_iter()
        .chain([[-1.0, 0.0, 0.0, 1.0]; 3])
        .collect::<Vec<_>>();
    let authored = tangent::prepare(
        &authored_vertices,
        &[0, 1, 2, 3, 4, 5],
        TangentSource::Authored(&authored_tangents),
    )?;
    let generated = tangent::prepare(&vertices, &indices, TangentSource::GenerateMikkTSpace)?;
    let pixels: Vec<_> = (0..3)
        .flat_map(|y| {
            (0..5).flat_map(move |x| match (x + y) % 4 {
                0 => [128, 128, 255, 255],
                1 => [204, 128, 230, 255],
                2 => [128, 204, 230, 255],
                _ => [51, 128, 230, 255],
            })
        })
        .collect();
    let normal_map = normal_map::prepare_rgba8(5, 3, &pixels)?;
    let mut buffer = Buffer {
        bytes: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };
    let authored_primitive = buffer.primitive(
        authored.vertices(),
        authored.indices(),
        Some(authored.tangents()),
    );
    let generated_primitive = buffer.primitive(&vertices, &indices, None);
    let image = buffer.view(&png(5, 3, &pixels)?);
    let root = json!({
        "asset":{"version":"2.0","generator":"Renderlab owned M5 normal preparation v1"},
        "scene":0,"scenes":[{"nodes":[0,1,2]}],
        "nodes":[{"mesh":0,"translation":[-1.5,0.0,0.0]},
            {"mesh":1,"translation":[1.5,0.0,0.0]},
            {"mesh":1,"translation":[0.0,-2.0,0.0],"scale":[-1.5,0.75,0.5]}],
        "meshes":[{"primitives":[authored_primitive]},{"primitives":[generated_primitive]}],
        "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.7,0.7,0.7,1.0],"metallicFactor":0.0,"roughnessFactor":0.4},
            "normalTexture":{"index":0,"texCoord":0,"scale":0.5},"doubleSided":true}],
        "textures":[{"source":0,"sampler":0}],
        "samplers":[{"magFilter":9729,"minFilter":9987,"wrapS":10497,"wrapT":10497}],
        "images":[{"bufferView":image,"mimeType":"image/png"}],
        "buffers":[{"byteLength":buffer.bytes.len()}],"bufferViews":buffer.views,"accessors":buffer.accessors,
        "extras":{"provenance":"Created for Renderlab. No third-party assets. Project license undecided.",
            "normalConvention":"linear RGB XYZ, +Y, material XY scale then normalize"}
    });
    Ok(NormalFixture {
        glb: crate::fixtures::frame(&root, &buffer.bytes)?,
        authored,
        generated,
        normal_map,
    })
}

/// Deterministic source GLB, encoded prepared mip PNGs, and counts for the v2 adapter.
pub fn generate(directory: &Path) -> Result<()> {
    let fixture = prepare()?;
    fs::create_dir_all(directory)?;
    fs::write(directory.join("normal-mapped.glb"), &fixture.glb)?;
    for (level, mip) in fixture.normal_map.mips().iter().enumerate() {
        fs::write(
            directory.join(format!("normal-mip-{level}.png")),
            png(mip.width(), mip.height(), &mip.rgba8())?,
        )?;
    }
    let info = json!({"fixtureRecipe":1,"normalScale":0.5,"normalTexCoord":0,
        "authoredVertices":fixture.authored.vertices().len(),
        "generatedVertices":fixture.generated.vertices().len(),
        "indicesPerMesh":fixture.generated.indices().len(),
        "mips":fixture.normal_map.mips().iter().map(|m| [m.width(),m.height()]).collect::<Vec<_>>()});
    fs::write(
        directory.join("preparation.json"),
        serde_json::to_vec_pretty(&info).map_err(|e| Error(e.to_string()))?,
    )?;
    Ok(())
}
