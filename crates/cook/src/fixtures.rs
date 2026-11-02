//! Deterministic self-authored geometry and swatches. No third-party artwork.
use crate::{Error, Result};
use image::{ExtendedColorType, ImageEncoder};
use renderlab_asset::Vertex;
use serde_json::{Value, json};
use std::{f32::consts::TAU, fs, path::Path};

struct Glb {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}
impl Glb {
    fn new() -> Self {
        Self {
            bin: Vec::new(),
            views: Vec::new(),
            accessors: Vec::new(),
        }
    }
    fn view(&mut self, bytes: &[u8], stride: Option<usize>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let mut view = json!({"buffer":0,"byteOffset":self.bin.len(),"byteLength":bytes.len()});
        if let Some(stride) = stride {
            view["byteStride"] = json!(stride);
        }
        let index = self.views.len();
        self.views.push(view);
        self.bin.extend(bytes);
        index
    }
    fn accessor(
        &mut self,
        view: usize,
        offset: usize,
        count: usize,
        component: u32,
        kind: &str,
        normalized: bool,
    ) -> usize {
        let index = self.accessors.len();
        let mut value = json!({"bufferView":view,"byteOffset":offset,"componentType":component,"count":count,"type":kind});
        if normalized {
            value["normalized"] = json!(true);
        }
        self.accessors.push(value);
        index
    }
    fn primitive(
        &mut self,
        vertices: &[Vertex],
        indices: Option<&[u32]>,
        material: usize,
        normals: bool,
    ) -> Value {
        let mut bytes = Vec::new();
        for v in vertices {
            for f in v.position.iter().chain(&v.normal) {
                bytes.extend(f.to_le_bytes());
            }
            // Fixture UVs fit [0,1]. Normalized accessors exercise integer conversion.
            for uv in v.uv {
                bytes.extend(((uv * 65535.0).round() as u16).to_le_bytes());
            }
            for color in v.color {
                bytes.push((color * 255.0).round() as u8);
            }
        }
        let view = self.view(&bytes, Some(32));
        let pos = self.accessor(view, 0, vertices.len(), 5126, "VEC3", false);
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for v in vertices {
            for i in 0..3 {
                min[i] = min[i].min(v.position[i]);
                max[i] = max[i].max(v.position[i]);
            }
        }
        self.accessors[pos]["min"] = json!(min);
        self.accessors[pos]["max"] = json!(max);
        let uv = self.accessor(view, 24, vertices.len(), 5123, "VEC2", true);
        let color = self.accessor(view, 28, vertices.len(), 5121, "VEC4", true);
        let mut attributes = json!({"POSITION":pos,"TEXCOORD_0":uv,"COLOR_0":color});
        if normals {
            attributes["NORMAL"] =
                json!(self.accessor(view, 12, vertices.len(), 5126, "VEC3", false));
        }
        let mut p = json!({"attributes":attributes,"material":material,"mode":4});
        if let Some(indices) = indices {
            let bytes: Vec<_> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            let view = self.view(&bytes, None);
            p["indices"] = json!(self.accessor(view, 0, indices.len(), 5125, "SCALAR", false));
        }
        p
    }
    fn png(&mut self, w: u32, h: u32) -> Result<Value> {
        let pixels = swatches(w, h);
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&pixels, w, h, ExtendedColorType::Rgba8)
            .map_err(|e| Error(e.to_string()))?;
        Ok(json!({"bufferView":self.view(&bytes,None),"mimeType":"image/png"}))
    }
    fn jpeg(&mut self, w: u32, h: u32) -> Result<Value> {
        let rgb: Vec<_> = swatches(w, h)
            .chunks_exact(4)
            .flat_map(|p| p[..3].iter().copied())
            .collect();
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
            .encode(&rgb, w, h, ExtendedColorType::Rgb8)
            .map_err(|e| Error(e.to_string()))?;
        Ok(json!({"bufferView":self.view(&bytes,None),"mimeType":"image/jpeg"}))
    }
    fn finish(self, mut root: Value) -> Result<Vec<u8>> {
        root["asset"] =
            json!({"version":"2.0","generator":"Renderlab self-authored M3 fixture v1"});
        root["buffers"] = json!([{"byteLength":self.bin.len()}]);
        root["bufferViews"] = json!(self.views);
        root["accessors"] = json!(self.accessors);
        frame(&root, &self.bin)
    }
}

pub(crate) fn frame(root: &Value, bin: &[u8]) -> Result<Vec<u8>> {
    let mut json = serde_json::to_vec(root).map_err(|e| Error(e.to_string()))?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut bytes = b"glTF".to_vec();
    bytes.extend(2u32.to_le_bytes());
    bytes.extend(((28 + json.len() + bin.len()) as u32).to_le_bytes());
    bytes.extend((json.len() as u32).to_le_bytes());
    bytes.extend(0x4e4f534au32.to_le_bytes());
    bytes.extend(json);
    bytes.extend((bin.len() as u32).to_le_bytes());
    bytes.extend(0x004e4942u32.to_le_bytes());
    bytes.extend(bin);
    Ok(bytes)
}
fn swatches(w: u32, h: u32) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let color = match (x + 2 * y) % 6 {
                0 => [0, 0, 0, 255],
                1 => [255, 255, 255, 255],
                2 => [255, 32, 16, 255],
                3 => [16, 255, 64, 255],
                4 => [64, 32, 255, 255],
                _ => [128, 128, 128, 255],
            };
            pixels.extend(color);
        }
    }
    pixels
}
fn root(primitives: Vec<Value>, images: Vec<Value>, nodes: Value) -> Value {
    json!({"scene":0,"scenes":[{"nodes":[0]}],"nodes":nodes,"meshes":[{"primitives":primitives}],"images":images,
        "samplers":[{"magFilter":9728,"minFilter":9984,"wrapS":33071,"wrapT":33648},
            {"magFilter":9729,"minFilter":9987,"wrapS":10497,"wrapT":10497}],
        "textures":[{"source":0,"sampler":0},{"source":0,"sampler":1},{"source":images.len()-1,"sampler":1}],
        "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[1.0,0.75,0.5,1.0],"metallicFactor":0.35,"roughnessFactor":0.65,
            "baseColorTexture":{"index":0},"metallicRoughnessTexture":{"index":1}}},
            {"doubleSided":true,"pbrMetallicRoughness":{"baseColorFactor":[0.5,0.75,1.0,1.0],"metallicFactor":0.8,"roughnessFactor":0.25,
                "baseColorTexture":{"index":2},"metallicRoughnessTexture":{"index":0}}}],
        "extras":{"provenance":"Created here for Renderlab. No third-party art. Project license undecided."}})
}

fn boxes() -> Result<Vec<u8>> {
    let mut glb = Glb::new();
    let mut groups: [Vec<Vertex>; 2] = [Vec::new(), Vec::new()];
    let mut indices: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
    let faces = [
        (
            [1.0, 0.0, 0.0],
            [
                [1.0, -1.0, -1.0],
                [1.0, 1.0, -1.0],
                [1.0, 1.0, 1.0],
                [1.0, -1.0, 1.0],
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [-1.0, -1.0, 1.0],
                [-1.0, 1.0, 1.0],
                [-1.0, 1.0, -1.0],
                [-1.0, -1.0, -1.0],
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [
                [-1.0, 1.0, -1.0],
                [-1.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, 1.0, -1.0],
            ],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [-1.0, -1.0, 1.0],
                [-1.0, -1.0, -1.0],
                [1.0, -1.0, -1.0],
                [1.0, -1.0, 1.0],
            ],
        ),
        (
            [0.0, 0.0, 1.0],
            [
                [1.0, -1.0, 1.0],
                [1.0, 1.0, 1.0],
                [-1.0, 1.0, 1.0],
                [-1.0, -1.0, 1.0],
            ],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                [-1.0, -1.0, -1.0],
                [-1.0, 1.0, -1.0],
                [1.0, 1.0, -1.0],
                [1.0, -1.0, -1.0],
            ],
        ),
    ];
    for (i, (normal, positions)) in faces.into_iter().enumerate() {
        let g = i % 2;
        let start = groups[g].len() as u32;
        for (position, uv) in
            positions
                .into_iter()
                .zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
        {
            groups[g].push(Vertex {
                position,
                normal,
                uv,
                color: if i < 2 {
                    [1.0; 4]
                } else {
                    [0.5, 1.0, 0.75, 1.0]
                },
            });
        }
        indices[g].extend([0, 1, 2, 0, 2, 3].map(|i| start + i));
    }
    let primitives = (0..2)
        .map(|i| glb.primitive(&groups[i], Some(&indices[i]), i, true))
        .collect();
    let image = glb.png(4, 4)?;
    let nodes = json!([{"translation":[-1.5,0.25,0.0],"children":[1,2,3]},
        {"mesh":0,"scale":[0.65,1.0,0.4]},
        {"mesh":0,"translation":[3.5,0.0,0.0],"scale":[-0.7,0.55,1.2],"children":[4]},
        {"translation":[0.0,2.5,0.0],"rotation":[0.0,0.0,0.38268343,0.9238795],"children":[5]},
        {"mesh":0,"translation":[0.0,3.0,0.0],"scale":[0.4,0.8,0.6]},
        {"mesh":0,"scale":[0.5,0.25,-0.5]}]);
    glb.finish(root(primitives, vec![image], nodes))
}

fn ribbon() -> Result<Vec<u8>> {
    let mut glb = Glb::new();
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    const SEGMENTS: usize = 96;
    const SIDES: usize = 12;
    for i in 0..=SEGMENTS {
        let u = i as f32 / SEGMENTS as f32;
        let angle = u * TAU;
        let (s, c) = angle.sin_cos();
        for j in 0..=SIDES {
            let v = j as f32 / SIDES as f32;
            let (ts, tc) = (v * TAU).sin_cos();
            let radius = 1.0 + 0.14 * tc;
            let wave = 0.15 * (angle * 3.0).sin();
            let tangent = [-s, c, 0.45 * (angle * 3.0).cos()];
            let across = [-ts * c, -ts * s, tc];
            let normal = [
                tangent[1] * across[2] - tangent[2] * across[1],
                tangent[2] * across[0] - tangent[0] * across[2],
                tangent[0] * across[1] - tangent[1] * across[0],
            ];
            let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt();
            vertices.push(Vertex {
                position: [radius * c, radius * s, wave + 0.14 * ts],
                normal: normal.map(|v| v / length),
                uv: [u, v],
                color: [1.0; 4],
            });
        }
    }
    for i in 0..SEGMENTS {
        for j in 0..SIDES {
            let a = (i * (SIDES + 1) + j) as u32;
            let b = a + (SIDES + 1) as u32;
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    let body = glb.primitive(&vertices, Some(&indices), 0, true);
    let mut fins = Vec::new();
    for i in 0..24 {
        let angle = i as f32 / 24.0 * TAU;
        let (s, c) = angle.sin_cos();
        for (r, t, uv) in [
            (1.05, -0.012, [0.0, 0.0]),
            (1.6, 0.0, [0.5, 1.0]),
            (1.05, 0.012, [1.0, 0.0]),
        ] {
            fins.push(Vertex {
                position: [c * r - s * t, s * r + c * t, 0.15 * (angle * 3.0).sin()],
                normal: [0.0, 0.0, 1.0],
                uv,
                color: [1.0; 4],
            });
        }
    }
    let thin = glb.primitive(&fins, None, 1, false);
    let images = vec![glb.png(5, 3)?, glb.jpeg(7, 5)?];
    glb.finish(root(
        vec![body, thin],
        images,
        json!([{"mesh":0,"children":[1]},
        {"mesh":0,"translation":[3.5,0.0,0.25],"scale":[-0.75,1.3,0.5]}]),
    ))
}

pub fn sources() -> Result<[(&'static str, Vec<u8>); 2]> {
    Ok([
        ("instanced-boxes.glb", boxes()?),
        ("curved-thin.glb", ribbon()?),
    ])
}
pub fn generate(directory: &Path) -> Result<()> {
    fs::create_dir_all(directory)?;
    for (name, bytes) in sources()? {
        fs::write(directory.join(name), bytes)?;
    }
    Ok(())
}
