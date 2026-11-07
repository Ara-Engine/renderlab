use renderlab_asset::{self as asset, TangentBasis, TangentStream, virtual_mesh as vg};
use renderlab_cook::{
    self as cook, CookLimits, Settings, normal_fixture, tangent, virtual_cache, virtual_cook,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn settings() -> Settings {
    Settings {
        normal_maps: true,
        normal_variance: true,
        ..Default::default()
    }
}
fn fixture() -> asset::Asset {
    cook::prepare(
        &normal_fixture::prepare().unwrap().glb,
        settings(),
        CookLimits::default(),
    )
    .unwrap()
}
fn mutate(change: impl FnOnce(&mut Value, &mut Vec<u8>)) -> Vec<u8> {
    let source = normal_fixture::prepare().unwrap().glb;
    let n = u32::from_le_bytes(source[12..16].try_into().unwrap()) as usize;
    let mut root: Value = serde_json::from_slice(&source[20..20 + n]).unwrap();
    let mut bin = source[28 + n..].to_vec();
    change(&mut root, &mut bin);
    let mut json = serde_json::to_vec(&root).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut out = b"glTF".to_vec();
    for word in [
        2,
        (28 + json.len() + bin.len()) as u32,
        json.len() as u32,
        0x4e4f534a,
    ] {
        out.extend(word.to_le_bytes());
    }
    out.extend(json);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(0x004e4942u32.to_le_bytes());
    out.extend(bin);
    out
}

#[test]
fn actual_owned_glb_imports_authored_and_mikk_frames_linear_mips_and_scale() {
    let source = normal_fixture::prepare().unwrap();
    assert!(cook::prepare(&source.glb, Settings::default(), CookLimits::default()).is_err());
    let a = fixture();
    for (i, expected, basis) in [
        (0, source.authored, TangentBasis::Authored),
        (1, source.generated, TangentBasis::MikkTSpace),
    ] {
        let p = &a.meshes[i].primitives[0];
        assert_eq!(p.vertices, expected.vertices());
        assert_eq!(p.indices, expected.indices());
        assert_eq!(p.tangents.as_ref().unwrap().values, expected.tangents());
        assert_eq!(p.tangents.as_ref().unwrap().basis, basis);
    }
    let m = a.materials[0].normal_texture.unwrap();
    assert_eq!(m.scale, 0.5);
    assert_eq!(m.sampler, asset::Sampler::default());
    let texture = &a.textures[m.texture as usize];
    assert_eq!(texture.role, asset::TextureRole::NormalVarianceLinear);
    for (m, expected) in texture.mips.iter().zip(source.normal_map.mips()) {
        assert_eq!(m.rgba8, expected.rgba8());
    }
    let bytes = asset::encode(&a, CookLimits::default().prepared).unwrap();
    assert_eq!(
        asset::decode(&bytes, CookLimits::default().prepared).unwrap(),
        a
    );
}

#[test]
fn missing_normals_ignore_authored_tangents_before_generating_the_frame() {
    for mapped in [false, true] {
        let source = mutate(|r, b| {
            let attributes = &mut r["meshes"][0]["primitives"][0]["attributes"];
            attributes.as_object_mut().unwrap().remove("NORMAL");
            let a = attributes["TANGENT"].as_u64().unwrap() as usize;
            let v = r["accessors"][a]["bufferView"].as_u64().unwrap() as usize;
            let offset = r["bufferViews"][v]["byteOffset"].as_u64().unwrap() as usize;
            let count = r["accessors"][a]["count"].as_u64().unwrap() as usize;
            for i in 0..count {
                for (j, value) in [0f32, 0., 1., 1.].into_iter().enumerate() {
                    let start = offset + i * 16 + j * 4;
                    b[start..start + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
            if !mapped {
                r["materials"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("normalTexture");
                r["meshes"][0]["primitives"][0]["attributes"]
                    .as_object_mut()
                    .unwrap()
                    .remove("TEXCOORD_0");
            }
        });
        let asset = cook::prepare(&source, settings(), CookLimits::default()).unwrap();
        let primitive = &asset.meshes[0].primitives[0];
        if mapped {
            assert_eq!(
                primitive.tangents.as_ref().unwrap().basis,
                TangentBasis::MikkTSpace
            );
            assert!(
                primitive
                    .tangents
                    .as_ref()
                    .unwrap()
                    .values
                    .iter()
                    .all(|t| t[2] == 0.)
            );
        } else {
            assert!(primitive.tangents.is_none());
        }
    }
}

#[test]
fn unsupported_normal_uv_transform_and_invalid_authored_frames_are_named() {
    for (source, needle) in [
        (
            mutate(|r, _| r["materials"][0]["normalTexture"]["texCoord"] = json!(1)),
            "coordinate",
        ),
        (
            mutate(|r, _| {
                r["materials"][0]["normalTexture"]["extensions"] =
                    json!({"KHR_texture_transform":{"offset":[0.1,0.0]}})
            }),
            "extensions",
        ),
        (
            mutate(|r, _| {
                r["meshes"][1]["primitives"][0]["attributes"]
                    .as_object_mut()
                    .unwrap()
                    .remove("TEXCOORD_0");
            }),
            "UV0",
        ),
        (
            mutate(|r, b| {
                let a = r["meshes"][0]["primitives"][0]["attributes"]["TANGENT"]
                    .as_u64()
                    .unwrap() as usize;
                let v = r["accessors"][a]["bufferView"].as_u64().unwrap() as usize;
                let offset = r["bufferViews"][v]["byteOffset"].as_u64().unwrap() as usize + 12;
                b[offset..offset + 4].copy_from_slice(&0f32.to_le_bytes());
            }),
            "tangent",
        ),
    ] {
        let e = cook::prepare(&source, settings(), CookLimits::default()).unwrap_err();
        assert!(e.0.contains(needle), "{e}");
    }
}

#[test]
fn finite_negative_zero_scale_and_exact_normal_sampler_survive_import() {
    for scale in [-2., 0., 1.] {
        let source = mutate(|r, _| {
            r["materials"][0]["normalTexture"]["scale"] = json!(scale);
            r["samplers"][0] =
                json!({"magFilter":9728,"minFilter":9984,"wrapS":33648,"wrapT":33071});
        });
        let a = cook::prepare(&source, settings(), CookLimits::default()).unwrap();
        let t = a.materials[0].normal_texture.unwrap();
        assert_eq!(t.scale, scale);
        assert_eq!(
            t.sampler,
            asset::Sampler {
                wrap_u: asset::Wrap::Mirror,
                wrap_v: asset::Wrap::Clamp,
                mag: asset::Filter::Nearest,
                min: asset::Filter::Nearest,
                mipmap: asset::MipmapFilter::Nearest
            }
        );
        assert_eq!(a.textures, fixture().textures);
    }
}

#[test]
fn intermediate_normal_mips_and_tangent_splits_respect_limits() {
    let source = normal_fixture::prepare().unwrap().glb;
    let limits = CookLimits {
        image_decoded_bytes: 200,
        ..Default::default()
    };
    assert!(
        cook::prepare(&source, settings(), limits)
            .unwrap_err()
            .0
            .contains("float preparation")
    );
    let limits = CookLimits {
        prepared: asset::Limits {
            max_vertices: 11,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(
        cook::prepare(&source, settings(), limits)
            .unwrap_err()
            .0
            .contains("tangent vertex limit")
    );
    let limits = CookLimits {
        prepared: asset::Limits {
            max_decoded_bytes: fixture()
                .validate(asset::Limits::default())
                .unwrap()
                .decoded_bytes,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(cook::prepare(&source, settings(), limits).is_err());
}

fn assert_pages(a: &asset::Asset, cooked: &virtual_cook::Cooked) {
    let limits = vg::VirtualLimits::default();
    let bytes = vg::encode(&cooked.package, limits).unwrap();
    assert_eq!(&bytes[..8], b"RLV\0\x02\0\0\0");
    assert_eq!(vg::decode(&bytes, limits).unwrap(), cooked.package);
    assert_eq!(cooked.package.resources.materials, a.materials);
    assert_eq!(cooked.package.resources.textures, a.textures);
    for (p, b) in cooked.package.pages.iter().zip(&cooked.pages) {
        assert_eq!(b.len(), vg::PAGE_BYTES);
        assert!(p.payload_bytes as usize <= vg::PAGE_BYTES);
        assert_eq!(p.payload_bytes, p.vertices * 64 + p.indices * 4);
        let g = vg::decode_page(p, b, Sha256::digest(b).into()).unwrap();
        let t = g.tangents.as_ref().unwrap();
        asset::validate_tangents(&g.vertices, &g.indices, t).unwrap();
        // Every packed frame remains paired with its original full attribute tuple.
        for (v, t) in g.vertices.iter().zip(&t.values) {
            assert!(a.meshes.iter().flat_map(|m| &m.primitives).any(|p| {
                p.vertices
                    .iter()
                    .zip(&p.tangents.as_ref().unwrap().values)
                    .any(|(sv, st)| sv == v && st == t)
            }));
        }
        let mut bad = b.clone();
        bad[p.vertices as usize * 48 + 12..p.vertices as usize * 48 + 16]
            .copy_from_slice(&0f32.to_le_bytes());
        let mut record = p.clone();
        record.sha256 = Sha256::digest(&bad).into();
        assert!(vg::decode_page(&record, &bad, record.sha256).is_err());
        let mut bad = b.clone();
        bad[p.vertices as usize * 64..p.vertices as usize * 64 + 4]
            .copy_from_slice(&p.vertices.to_le_bytes());
        record.sha256 = Sha256::digest(&bad).into();
        assert!(vg::decode_page(&record, &bad, record.sha256).is_err());
    }
    let n = cooked.package.pages.len() * vg::PAGE_BYTES;
    assert!(
        cooked
            .package
            .validate(vg::VirtualLimits {
                max_geometry_bytes: n - 1,
                ..limits
            })
            .is_err()
    );
    let mut bad = cooked.package.clone();
    bad.pages[0].tangents = None;
    assert!(bad.validate(limits).is_err());
}

#[test]
fn normal_bearing_virtual_pages_preserve_mirrors_materials_and_accounting() {
    let a = fixture();
    let cooked = virtual_cook::prepare(
        &a,
        virtual_cook::Settings::default(),
        vg::VirtualLimits::default(),
    )
    .unwrap();
    assert_pages(&a, &cooked);
    for mesh in &cooked.package.meshes {
        for cluster in &mesh.clusters {
            assert_eq!(cluster.material, 0);
        }
    }
}

#[test]
fn dense_normal_surface_simplification_preserves_complete_source_frames_and_materials() {
    let mut a = cook::virtual_fixtures::closed_sphere(12).unwrap();
    for p in &mut a.meshes[0].primitives {
        let frames = tangent::prepare(
            &p.vertices,
            &p.indices,
            tangent::TangentSource::GenerateMikkTSpace,
        )
        .unwrap();
        let (vertices, indices, values) = frames.into_parts();
        p.vertices = vertices;
        p.indices = indices;
        p.tangents = Some(TangentStream {
            basis: TangentBasis::MikkTSpace,
            values,
        });
    }
    let normal = fixture();
    a.textures = normal.textures;
    a.materials[0] = normal.materials[0].clone();
    let mut second = a.meshes[0].primitives[0].clone();
    second.material = 1;
    for v in &mut second.vertices {
        v.position[0] += 3.;
        v.uv[0] = -v.uv[0];
    }
    for t in &mut second.tangents.as_mut().unwrap().values {
        t[0] = -t[0];
        t[1] = -t[1];
        t[2] = -t[2];
        t[3] = -t[3];
    }
    a.materials.push(a.materials[0].clone());
    a.materials[1].base_color = [0.2, 0.4, 0.8, 1.];
    a.meshes[0].primitives.push(second);
    let cooked = virtual_cook::prepare(
        &a,
        virtual_cook::Settings::default(),
        vg::VirtualLimits::default(),
    )
    .unwrap();
    assert!(cooked.package.meshes[0].groups.iter().any(|g| g.depth > 0));
    assert_pages(&a, &cooked);
    for c in &cooked.package.meshes[0].clusters {
        let record = &cooked.package.pages[c.page.index()];
        let b = &cooked.pages[c.page.index()];
        let g = vg::decode_page(record, b, Sha256::digest(b).into()).unwrap();
        for &i in &g.indices[c.indices.range(g.indices.len()).unwrap()] {
            let x = g.vertices[i as usize].position[0];
            assert!(if c.material == 0 { x <= 1. } else { x >= 2. });
        }
    }
}

#[test]
fn normal_cache_namespaces_roundtrip_warm_and_repair_without_rewriting_legacy_keys() {
    let directory =
        std::env::temp_dir().join(format!("renderlab-normal-v2-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("normal.glb");
    let current = directory.join("current");
    let virtual_current = directory.join("virtual-current");
    std::fs::write(&source, normal_fixture::prepare().unwrap().glb).unwrap();
    let cold = cook::cook_file(
        &source,
        &directory,
        &current,
        settings(),
        CookLimits::default(),
    )
    .unwrap();
    assert!(!cold.report.cache_hit);
    let warm = cook::cook_file(
        &source,
        &directory,
        &current,
        settings(),
        CookLimits::default(),
    )
    .unwrap();
    assert!(warm.report.cache_hit);
    assert_eq!(cold.loaded.asset, warm.loaded.asset);
    assert_eq!(&std::fs::read(&current).unwrap()[..8], b"RLM\0\x02\0\0\0");
    assert!(directory.join("normal-v3/keys").is_dir());
    let legacy = cook::cook_file(
        &source,
        &directory,
        &directory.join("legacy-current"),
        Settings {
            normal_variance: false,
            ..settings()
        },
        CookLimits::default(),
    )
    .unwrap();
    assert_ne!(legacy.loaded.key, cold.loaded.key);
    assert!(directory.join("normal-v2/keys").is_dir());
    for texture in &legacy.loaded.asset.textures {
        assert_eq!(texture.role, asset::TextureRole::NormalLinear);
        assert!(
            texture
                .mips
                .iter()
                .all(|m| m.rgba8.chunks_exact(4).all(|p| p[3] == 255))
        );
    }
    assert_eq!(
        &asset::encode(&legacy.loaded.asset, CookLimits::default().prepared).unwrap()[..8],
        b"RLA\0\x02\0\0\0"
    );
    let rla = directory.join("normal.rla");
    std::fs::write(
        &rla,
        asset::encode(&cold.loaded.asset, CookLimits::default().prepared).unwrap(),
    )
    .unwrap();
    let first = virtual_cache::cook_file(
        &rla,
        &directory,
        &virtual_current,
        virtual_cook::Settings::default(),
        vg::VirtualLimits::default(),
    )
    .unwrap();
    assert!(!first.report.cache_hit);
    let warm = virtual_cache::cook_file(
        &rla,
        &directory,
        &virtual_current,
        virtual_cook::Settings::default(),
        vg::VirtualLimits::default(),
    )
    .unwrap();
    assert!(warm.report.cache_hit);
    assert_eq!(
        &std::fs::read(&virtual_current).unwrap()[..8],
        b"RVM\0\x02\0\0\0"
    );
    for p in &first.loaded.package.pages {
        virtual_cache::load_page(&directory, p).unwrap();
    }
    let p = &first.loaded.package.pages[0];
    std::fs::write(virtual_cache::page_path(&directory, p), [0]).unwrap();
    let repaired = virtual_cache::cook_file(
        &rla,
        &directory,
        &virtual_current,
        virtual_cook::Settings::default(),
        vg::VirtualLimits::default(),
    )
    .unwrap();
    assert!(!repaired.report.cache_hit);
    assert_eq!(repaired.loaded.package, first.loaded.package);
    std::fs::remove_dir_all(directory).unwrap();
}
