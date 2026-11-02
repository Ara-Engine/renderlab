use crate::{Error, Result, ensure, mip};
use gltf::{
    accessor::{DataType, Dimensions},
    mesh::Semantic,
};
use renderlab_asset::*;
use std::{collections::HashMap, io::Cursor};

#[derive(Clone, Copy, Debug)]
pub struct CookLimits {
    pub source_bytes: usize,
    pub json_bytes: usize,
    pub nodes: usize,
    pub hierarchy_depth: usize,
    pub images: usize,
    pub image_decoded_bytes: usize,
    pub prepared: Limits,
}
impl Default for CookLimits {
    fn default() -> Self {
        Self {
            source_bytes: 64 << 20,
            json_bytes: 8 << 20,
            nodes: 16384,
            hierarchy_depth: 128,
            images: 256,
            image_decoded_bytes: 128 << 20,
            prepared: Limits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MissingNormals {
    #[default]
    Flat,
    Reject,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Settings {
    pub missing_normals: MissingNormals,
}
impl Settings {
    pub(crate) fn bytes(self) -> [u8; 1] {
        [match self.missing_normals {
            MissingNormals::Flat => 0,
            MissingNormals::Reject => 1,
        }]
    }
}

/// GLB framing is parsed before either JSON parser. Unknown chunks are rejected.
pub(crate) fn chunks(source: &[u8], limits: CookLimits) -> Result<(&[u8], &[u8])> {
    ensure(
        source.len() <= limits.source_bytes,
        "source file byte limit",
    )?;
    ensure(source.len() >= 20, "truncated GLB header")?;
    let word = |i| u32::from_le_bytes(source[i..i + 4].try_into().unwrap()) as usize;
    ensure(
        &source[..4] == b"glTF" && word(4) == 2,
        "only GLB 2.0 is supported",
    )?;
    ensure(
        word(8) == source.len(),
        "GLB declared length mismatch or trailing bytes",
    )?;
    ensure(word(16) == 0x4e4f534a, "GLB first chunk must be JSON")?;
    let n = word(12);
    ensure(
        n.is_multiple_of(4) && n <= limits.json_bytes,
        "GLB JSON length/alignment limit",
    )?;
    let json_end = 20usize
        .checked_add(n)
        .ok_or_else(|| Error("JSON size overflow".into()))?;
    let json = source
        .get(20..json_end)
        .ok_or_else(|| Error("truncated GLB JSON".into()))?;
    ensure(
        source.len().saturating_sub(json_end) >= 8,
        "missing embedded BIN chunk",
    )?;
    let bin_len = word(json_end);
    let kind = word(json_end + 4);
    ensure(
        kind == 0x004e4942 && bin_len.is_multiple_of(4),
        "only aligned embedded BIN chunk supported",
    )?;
    let end = json_end
        .checked_add(8)
        .and_then(|v| v.checked_add(bin_len))
        .ok_or_else(|| Error("BIN size overflow".into()))?;
    ensure(
        end == source.len(),
        "truncated BIN, extra chunks or trailing GLB bytes",
    )?;
    Ok((json, &source[json_end + 8..end]))
}

fn preflight(json: &[u8], limits: CookLimits) -> Result<()> {
    let root: serde_json::Value =
        serde_json::from_slice(json).map_err(|e| Error(format!("GLB JSON: {e}")))?;
    ensure(
        root["asset"]["version"] == "2.0"
            && root["asset"].get("minVersion").is_none_or(|v| v == "2.0"),
        "only glTF asset version 2.0 supported",
    )?;
    for (key, cap) in [
        ("nodes", limits.nodes),
        ("meshes", limits.prepared.max_meshes),
        ("materials", limits.prepared.max_materials),
        ("textures", limits.prepared.max_textures),
        ("images", limits.images),
        ("accessors", 65536),
        ("bufferViews", 65536),
        ("scenes", 256),
        ("samplers", 4096),
    ] {
        if let Some(v) = root.get(key) {
            ensure(
                v.as_array().is_some_and(|a| a.len() <= cap),
                format!("{key} count/type limit"),
            )?;
        }
    }
    for key in ["skins", "animations"] {
        ensure(
            root.get(key)
                .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty)),
            format!("unsupported {key}"),
        )?;
    }
    for key in ["extensionsUsed", "extensionsRequired"] {
        ensure(
            root.get(key)
                .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty)),
            format!("unsupported {key}: {}", root[key]),
        )?;
    }
    if let Some(nodes) = root["nodes"].as_array() {
        let mut edges = 0usize;
        for node in nodes {
            ensure(
                node.get("matrix").is_none()
                    || ["translation", "rotation", "scale"]
                        .iter()
                        .all(|key| node.get(key).is_none()),
                "node cannot mix matrix and TRS",
            )?;
            if let Some(rotation) = node.get("rotation").and_then(serde_json::Value::as_array) {
                let length = rotation
                    .iter()
                    .map(|v| v.as_f64().unwrap_or(f64::NAN).powi(2))
                    .sum::<f64>();
                ensure(
                    rotation.len() == 4 && (length - 1.0).abs() <= 1e-4,
                    "node rotation must be a unit quaternion",
                )?;
            }
            if let Some(children) = node.get("children").and_then(serde_json::Value::as_array) {
                edges = edges
                    .checked_add(children.len())
                    .ok_or_else(|| Error("node edge overflow".into()))?;
                ensure(edges <= limits.nodes, "node hierarchy edge limit")?;
            }
        }
    }
    let mut stack = vec![&root];
    let mut visited = 0;
    while let Some(value) = stack.pop() {
        visited += 1;
        ensure(visited <= 500_000, "JSON structure work limit")?;
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    if key == "extensions" {
                        ensure(
                            value.as_object().is_some_and(serde_json::Map::is_empty),
                            format!("unsupported extensions: {value}"),
                        )?;
                    } else if key != "extras" {
                        stack.push(value);
                    }
                }
            }
            serde_json::Value::Array(values) => stack.extend(values),
            _ => {}
        }
    }
    Ok(())
}

fn validate_accessors(document: &gltf::Document, blob: &[u8], limits: CookLimits) -> Result<()> {
    let buffers: Vec<_> = document.buffers().collect();
    ensure(buffers.len() == 1, "exactly one embedded buffer required")?;
    ensure(
        matches!(buffers[0].source(), gltf::buffer::Source::Bin),
        "unsupported buffer URI; embedded BIN required",
    )?;
    let len = buffers[0].length();
    ensure(
        len <= blob.len() && blob.len() - len <= 3,
        "embedded buffer length mismatch",
    )?;
    for view in document.views() {
        ensure(view.buffer().index() == 0, "invalid buffer view resource")?;
        ensure(
            view.offset()
                .checked_add(view.length())
                .is_some_and(|end| end <= len),
            format!("bufferView {} span outside BIN", view.index()),
        )?;
        if let Some(stride) = view.stride() {
            ensure(
                (4..=252).contains(&stride) && stride.is_multiple_of(4),
                "invalid bufferView stride",
            )?;
        }
    }
    let mut total = 0usize;
    for a in document.accessors() {
        ensure(a.sparse().is_none(), "unsupported sparse accessor")?;
        let view = a
            .view()
            .ok_or_else(|| Error(format!("accessor {} missing bufferView", a.index())))?;
        ensure(
            a.count() > 0 && a.count() <= limits.prepared.max_indices,
            "accessor count limit",
        )?;
        ensure(
            matches!(
                a.dimensions(),
                Dimensions::Scalar | Dimensions::Vec2 | Dimensions::Vec3 | Dimensions::Vec4
            ),
            "unsupported matrix accessor",
        )?;
        let size = a.size();
        let stride = view.stride().unwrap_or(size);
        let scalar = a.data_type().size();
        ensure(
            stride >= size
                && stride.is_multiple_of(scalar)
                && a.offset().is_multiple_of(scalar)
                && view
                    .offset()
                    .checked_add(a.offset())
                    .is_some_and(|n| n.is_multiple_of(scalar)),
            "invalid accessor alignment or stride",
        )?;
        let end = (a.count() - 1)
            .checked_mul(stride)
            .and_then(|v| v.checked_add(a.offset()))
            .and_then(|v| v.checked_add(size));
        ensure(
            end.is_some_and(|v| v <= view.length()),
            format!("accessor {} span outside bufferView", a.index()),
        )?;
        total = total
            .checked_add(
                a.count()
                    .checked_mul(size)
                    .ok_or_else(|| Error("accessor byte overflow".into()))?,
            )
            .ok_or_else(|| Error("accessor byte overflow".into()))?;
        ensure(
            total <= limits.prepared.max_decoded_bytes,
            "accessor aggregate byte limit",
        )?;
    }
    Ok(())
}

fn sampler(s: gltf::texture::Sampler<'_>) -> Sampler {
    use gltf::texture::{MagFilter as Mag, MinFilter as Min, WrappingMode as W};
    let wrap = |v| match v {
        W::ClampToEdge => Wrap::Clamp,
        W::MirroredRepeat => Wrap::Mirror,
        W::Repeat => Wrap::Repeat,
    };
    let (min, mipmap) = match s.min_filter() {
        Some(Min::Nearest) => (Filter::Nearest, MipmapFilter::None),
        Some(Min::Linear) => (Filter::Linear, MipmapFilter::None),
        Some(Min::NearestMipmapNearest) => (Filter::Nearest, MipmapFilter::Nearest),
        Some(Min::LinearMipmapNearest) => (Filter::Linear, MipmapFilter::Nearest),
        Some(Min::NearestMipmapLinear) => (Filter::Nearest, MipmapFilter::Linear),
        Some(Min::LinearMipmapLinear) | None => (Filter::Linear, MipmapFilter::Linear),
    };
    Sampler {
        wrap_u: wrap(s.wrap_s()),
        wrap_v: wrap(s.wrap_t()),
        mag: match s.mag_filter() {
            Some(Mag::Nearest) => Filter::Nearest,
            _ => Filter::Linear,
        },
        min,
        mipmap,
    }
}

struct Textures<'a> {
    blob: &'a [u8],
    limits: CookLimits,
    decoded: usize,
    prepared: usize,
    records: Vec<Texture>,
    roles: HashMap<(usize, TextureRole), u32>,
}
impl Textures<'_> {
    fn reference(
        &mut self,
        info: gltf::texture::Info<'_>,
        role: TextureRole,
    ) -> Result<(u32, Sampler)> {
        ensure(
            info.tex_coord() == 0,
            "unsupported texture coordinate set; only UV0",
        )?;
        let source = info.texture().source();
        let s = sampler(info.texture().sampler());
        if let Some(&index) = self.roles.get(&(source.index(), role)) {
            return Ok((index, s));
        }
        ensure(
            self.records.len() < self.limits.prepared.max_textures,
            "prepared texture role count limit",
        )?;
        let (view, mime) = match source.source() {
            gltf::image::Source::View { view, mime_type } => (view, mime_type),
            gltf::image::Source::Uri { .. } => {
                return Err(Error(
                    "unsupported image URI; embedded PNG/JPEG required".into(),
                ));
            }
        };
        let format = match mime {
            "image/png" => image::ImageFormat::Png,
            "image/jpeg" => image::ImageFormat::Jpeg,
            _ => return Err(Error(format!("unsupported image MIME {mime}"))),
        };
        let bytes = &self.blob[view.offset()..view.offset() + view.length()];
        ensure(
            image::guess_format(bytes).ok() == Some(format),
            "image MIME/signature mismatch",
        )?;
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
        let mut decoder_limits = image::Limits::default();
        decoder_limits.max_image_width = Some(self.limits.prepared.max_dimension);
        decoder_limits.max_image_height = Some(self.limits.prepared.max_dimension);
        decoder_limits.max_alloc = Some(self.limits.image_decoded_bytes as u64);
        reader.limits(decoder_limits);
        use image::ImageDecoder;
        let decoder = reader
            .into_decoder()
            .map_err(|e| Error(format!("image header/limits: {e}")))?;
        let (w, h) = decoder.dimensions();
        ensure(
            w > 0
                && h > 0
                && w <= self.limits.prepared.max_dimension
                && h <= self.limits.prepared.max_dimension,
            "image dimension limit",
        )?;
        let rgba = u64::from(w) * u64::from(h) * 4;
        let native = decoder.total_bytes();
        let charge = native
            .checked_add(rgba)
            .ok_or_else(|| Error("image size overflow".into()))?;
        ensure(
            charge <= self.limits.image_decoded_bytes.saturating_sub(self.decoded) as u64,
            "aggregate decoded image byte limit",
        )?;
        self.decoded += charge as usize;
        let mut mip_bytes = 0u64;
        let (mut mw, mut mh) = (w, h);
        loop {
            mip_bytes += u64::from(mw) * u64::from(mh) * 4;
            if mw == 1 && mh == 1 {
                break;
            }
            mw = (mw / 2).max(1);
            mh = (mh / 2).max(1);
        }
        ensure(
            mip_bytes
                <= self
                    .limits
                    .prepared
                    .max_decoded_bytes
                    .saturating_sub(self.prepared) as u64,
            "prepared mip byte limit",
        )?;
        self.prepared += mip_bytes as usize;
        let rgba = image::DynamicImage::from_decoder(decoder)
            .map_err(|e| Error(format!("image decode: {e}")))?
            .to_rgba8();
        let index = self.records.len() as u32;
        self.records.push(mip::build(w, h, rgba.into_raw(), role)?);
        self.roles.insert((source.index(), role), index);
        Ok((index, s))
    }
}

fn materials(document: &gltf::Document, textures: &mut Textures<'_>) -> Result<Vec<Material>> {
    let mut result = Vec::new();
    for m in document.materials() {
        ensure(
            m.alpha_mode() == gltf::material::AlphaMode::Opaque,
            "unsupported material alphaMode",
        )?;
        ensure(
            m.normal_texture().is_none(),
            "unsupported normalTexture/tangents",
        )?;
        ensure(
            m.occlusion_texture().is_none(),
            "unsupported occlusionTexture",
        )?;
        ensure(
            m.emissive_texture().is_none() && m.emissive_factor() == [0.0; 3],
            "unsupported emissive material",
        )?;
        let p = m.pbr_metallic_roughness();
        let base_color_texture = p
            .base_color_texture()
            .map(|info| {
                textures
                    .reference(info, TextureRole::BaseColorSrgb)
                    .map(|(texture, sampler)| BaseColorTexture { texture, sampler })
            })
            .transpose()?;
        let metallic_roughness_texture = p
            .metallic_roughness_texture()
            .map(|info| {
                textures
                    .reference(info, TextureRole::MetallicRoughnessLinear)
                    .map(|(texture, sampler)| MetallicRoughnessTexture { texture, sampler })
            })
            .transpose()?;
        result.push(Material {
            base_color: p.base_color_factor(),
            metallic: p.metallic_factor(),
            roughness: p.roughness_factor(),
            double_sided: m.double_sided(),
            base_color_texture,
            metallic_roughness_texture,
        });
    }
    Ok(result)
}

fn attribute(a: &gltf::Accessor<'_>, semantic: &Semantic, vertices: usize) -> Result<()> {
    ensure(a.count() == vertices, "inconsistent attribute counts")?;
    let float = a.data_type() == DataType::F32 && !a.normalized();
    let normalized = matches!(a.data_type(), DataType::U8 | DataType::U16) && a.normalized();
    let valid = match semantic {
        Semantic::Positions | Semantic::Normals => a.dimensions() == Dimensions::Vec3 && float,
        Semantic::TexCoords(0) => a.dimensions() == Dimensions::Vec2 && (float || normalized),
        Semantic::Colors(0) => {
            matches!(a.dimensions(), Dimensions::Vec3 | Dimensions::Vec4) && (float || normalized)
        }
        _ => false,
    };
    ensure(
        valid,
        format!("unsupported attribute/type/normalization {semantic:?}"),
    )
}

fn meshes(
    document: &gltf::Document,
    blob: &[u8],
    materials: &mut Vec<Material>,
    settings: Settings,
    limits: CookLimits,
) -> Result<Vec<Mesh>> {
    let mut result = Vec::new();
    let (mut nv, mut ni, mut np) = (0usize, 0usize, 0usize);
    let mut default_material = None;
    for mesh in document.meshes() {
        ensure(mesh.weights().is_none(), "unsupported mesh morph weights")?;
        let mut output = Mesh {
            primitives: Vec::new(),
        };
        for p in mesh.primitives() {
            np += 1;
            ensure(
                np <= limits.prepared.max_primitives,
                "primitive count limit",
            )?;
            ensure(
                p.mode() == gltf::mesh::Mode::Triangles,
                "unsupported primitive topology; triangles required",
            )?;
            ensure(
                p.morph_targets().next().is_none(),
                "unsupported morph targets",
            )?;
            let positions = p
                .get(&Semantic::Positions)
                .ok_or_else(|| Error("missing POSITION accessor".into()))?;
            let count = positions.count();
            ensure(
                count <= limits.prepared.max_vertices,
                "primitive vertex limit",
            )?;
            for (s, a) in p.attributes() {
                attribute(&a, &s, count)?;
            }
            let index_count = p.indices().map_or(count, |a| a.count());
            ensure(
                index_count > 0 && index_count.is_multiple_of(3),
                "triangle index count must be nonzero multiple of 3",
            )?;
            if let Some(a) = p.indices() {
                ensure(
                    a.dimensions() == Dimensions::Scalar
                        && matches!(a.data_type(), DataType::U8 | DataType::U16 | DataType::U32)
                        && !a.normalized()
                        && a.view().is_some_and(|v| v.stride().is_none()),
                    "unsupported index accessor type/stride",
                )?;
            }
            let flat = p.get(&Semantic::Normals).is_none();
            ensure(
                !flat || settings.missing_normals == MissingNormals::Flat,
                "missing NORMAL; flat generation disabled",
            )?;
            nv = nv
                .checked_add(if flat { index_count } else { count })
                .ok_or_else(|| Error("vertex overflow".into()))?;
            ni = ni
                .checked_add(index_count)
                .ok_or_else(|| Error("index overflow".into()))?;
            ensure(
                nv <= limits.prepared.max_vertices && ni <= limits.prepared.max_indices,
                "aggregate geometry count limit",
            )?;
            ensure(
                nv.checked_mul(48)
                    .and_then(|n| n.checked_add(ni.checked_mul(4)?))
                    .is_some_and(|n| n <= limits.prepared.max_decoded_bytes),
                "aggregate geometry byte limit",
            )?;
            let material = match p.material().index() {
                Some(i) => i as u32,
                None => *default_material.get_or_insert_with(|| {
                    let i = materials.len() as u32;
                    materials.push(Material::default());
                    i
                }),
            };
            ensure(
                materials.len() <= limits.prepared.max_materials,
                "material count limit including default",
            )?;
            let textured = materials[material as usize].base_color_texture.is_some()
                || materials[material as usize]
                    .metallic_roughness_texture
                    .is_some();
            ensure(
                !textured || p.get(&Semantic::TexCoords(0)).is_some(),
                "textured primitive requires UV0",
            )?;
            let reader = p.reader(|buffer| {
                if buffer.index() == 0 {
                    Some(blob)
                } else {
                    None
                }
            });
            let positions = reader
                .read_positions()
                .ok_or_else(|| Error("cannot read POSITION".into()))?;
            let mut normals = reader.read_normals();
            let mut uv = reader.read_tex_coords(0).map(|v| v.into_f32());
            let mut colors = reader.read_colors(0).map(|v| v.into_rgba_f32());
            let mut vertices = Vec::with_capacity(count);
            for position in positions {
                vertices.push(Vertex {
                    position,
                    normal: normals
                        .as_mut()
                        .and_then(Iterator::next)
                        .unwrap_or([0.0, 0.0, 1.0]),
                    uv: uv.as_mut().and_then(Iterator::next).unwrap_or([0.0; 2]),
                    color: colors.as_mut().and_then(Iterator::next).unwrap_or([1.0; 4]),
                });
            }
            ensure(vertices.len() == count, "truncated position reader")?;
            let indices: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..count as u32).collect(),
            };
            ensure(
                indices.len() == index_count
                    && indices.iter().all(|&i| (i as usize) < vertices.len()),
                "invalid primitive vertex index or truncated indices",
            )?;
            let (vertices, indices) = if flat {
                flat_normals(&vertices, &indices)?
            } else {
                (vertices, indices)
            };
            output.primitives.push(Primitive {
                vertices,
                indices,
                material,
            });
        }
        result.push(output);
    }
    Ok(result)
}

fn flat_normals(vertices: &[Vertex], indices: &[u32]) -> Result<(Vec<Vertex>, Vec<u32>)> {
    let mut output = Vec::with_capacity(indices.len());
    for tri in indices.chunks_exact(3) {
        let a = vertices[tri[0] as usize];
        let b = vertices[tri[1] as usize];
        let c = vertices[tri[2] as usize];
        let mut u = [0.0; 3];
        let mut v = [0.0; 3];
        for i in 0..3 {
            u[i] = f64::from(b.position[i]) - f64::from(a.position[i]);
            v[i] = f64::from(c.position[i]) - f64::from(a.position[i]);
        }
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let len = n.iter().map(|v| v * v).sum::<f64>().sqrt();
        ensure(
            len.is_finite() && len > 0.0,
            "degenerate triangle cannot generate flat normal",
        )?;
        let normal = n.map(|v| (v / len) as f32);
        for mut vertex in [a, b, c] {
            vertex.normal = normal;
            output.push(vertex);
        }
    }
    Ok((output, (0..indices.len() as u32).collect()))
}

pub(crate) const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];
fn multiply(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut m = [[0.0; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            m[c][r] = (0..4)
                .map(|k| f64::from(a[k][r]) * f64::from(b[c][k]))
                .sum::<f64>() as f32;
        }
    }
    m
}

fn placements(document: &gltf::Document, limits: CookLimits) -> Result<Vec<Placement>> {
    let nodes: Vec<_> = document.nodes().collect();
    let mut parents = vec![None; nodes.len()];
    for node in &nodes {
        ensure(
            node.skin().is_none() && node.weights().is_none(),
            "unsupported node skin/morph weights",
        )?;
        ensure(node.camera().is_none(), "unsupported camera node")?;
        ensure(
            valid_transform(&node.transform().matrix()),
            format!(
                "node {} requires finite nonsingular affine transform",
                node.index()
            ),
        )?;
        for child in node.children() {
            ensure(
                parents[child.index()].replace(node.index()).is_none(),
                "invalid hierarchy: node has multiple parents or repeated edge",
            )?;
        }
    }
    // Kahn traversal validates even nodes outside the selected scene and bounds depth.
    let mut queue: Vec<_> = parents
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.is_none().then_some((i, 1usize)))
        .collect();
    let mut cursor = 0;
    while cursor < queue.len() {
        let (i, depth) = queue[cursor];
        cursor += 1;
        ensure(
            depth <= limits.hierarchy_depth,
            "node hierarchy depth limit",
        )?;
        for child in nodes[i].children() {
            queue.push((child.index(), depth + 1));
        }
    }
    ensure(queue.len() == nodes.len(), "invalid hierarchy cycle")?;
    let scene = document
        .default_scene()
        .or_else(|| {
            let mut scenes = document.scenes();
            let first = scenes.next();
            if scenes.next().is_none() { first } else { None }
        })
        .ok_or_else(|| Error("one scene or explicit default scene required".into()))?;
    let mut seen = vec![false; nodes.len()];
    let mut stack = Vec::new();
    for root in scene.nodes() {
        ensure(parents[root.index()].is_none(), "scene root has a parent")?;
        stack.push((root.index(), IDENTITY));
    }
    stack.reverse();
    let mut result = Vec::new();
    while let Some((i, parent)) = stack.pop() {
        ensure(
            !std::mem::replace(&mut seen[i], true),
            "duplicate scene node",
        )?;
        let transform = multiply(parent, nodes[i].transform().matrix());
        ensure(
            valid_transform(&transform),
            "composed transform is nonfinite, singular or nonaffine",
        )?;
        if let Some(mesh) = nodes[i].mesh() {
            ensure(
                result.len() < limits.prepared.max_placements,
                "placement count limit",
            )?;
            result.push(Placement {
                mesh: mesh.index() as u32,
                transform,
            });
        }
        let children: Vec<_> = nodes[i]
            .children()
            .map(|c| (c.index(), transform))
            .collect();
        stack.extend(children.into_iter().rev());
    }
    Ok(result)
}

/// Prepare only the documented static embedded subset. No resource URI is resolved.
pub fn prepare(source: &[u8], settings: Settings, limits: CookLimits) -> Result<Asset> {
    let (json, blob) = chunks(source, limits)?;
    preflight(json, limits)?;
    let gltf =
        gltf::Gltf::from_slice(source).map_err(|e| Error(format!("glTF validation: {e}")))?;
    let document = &gltf.document;
    validate_accessors(document, blob, limits)?;
    for image in document.images() {
        ensure(
            matches!(
                image.source(),
                gltf::image::Source::View {
                    mime_type: "image/png" | "image/jpeg",
                    ..
                }
            ),
            "unsupported image URI/MIME; embedded PNG/JPEG required",
        )?;
    }
    let placements = placements(document, limits)?;
    let mut textures = Textures {
        blob,
        limits,
        decoded: 0,
        prepared: 0,
        records: Vec::new(),
        roles: HashMap::new(),
    };
    let mut materials = materials(document, &mut textures)?;
    let other_bytes = textures
        .prepared
        .checked_add(
            materials
                .len()
                .saturating_add(1)
                .saturating_mul(std::mem::size_of::<Material>()),
        )
        .and_then(|n| {
            n.checked_add(
                placements
                    .len()
                    .saturating_mul(std::mem::size_of::<Placement>()),
            )
        })
        .ok_or_else(|| Error("prepared byte overflow".into()))?;
    let mut geometry_limits = limits;
    geometry_limits.prepared.max_decoded_bytes = limits
        .prepared
        .max_decoded_bytes
        .checked_sub(other_bytes)
        .ok_or_else(|| Error("aggregate prepared byte limit".into()))?;
    let meshes = meshes(document, blob, &mut materials, settings, geometry_limits)?;
    let asset = Asset {
        meshes,
        placements,
        materials,
        textures: textures.records,
    };
    asset.validate(limits.prepared)?;
    Ok(asset)
}
