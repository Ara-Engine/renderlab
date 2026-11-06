use renderlab_asset::*;

fn triangle() -> Asset {
    let vertices = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
        .map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
            color: [1.; 4],
        })
        .to_vec();
    Asset {
        meshes: vec![Mesh {
            primitives: vec![Primitive {
                adjacency: None,
                vertices,
                indices: vec![0, 1, 2],
                material: 0,
                tangents: None,
            }],
        }],
        materials: vec![Material::default()],
        ..Default::default()
    }
}

#[test]
fn adjacency_v3_roundtrips_with_and_without_normal_maps_and_rejects_corruption() {
    for mut asset in [triangle(), mapped()] {
        let primitive = &mut asset.meshes[0].primitives[0];
        primitive.adjacency =
            Some(adjacency::build(&primitive.vertices, &primitive.indices).neighbours);
        let limits = Limits::default();
        let bytes = encode(&asset, limits).unwrap();
        assert_eq!(&bytes[..8], b"RLA\0\x03\0\0\0");
        assert_eq!(decode(&bytes, limits).unwrap(), asset);
        assert_eq!(encode(&asset, limits).unwrap(), bytes);
        assert_eq!(
            &encode_mesh(&asset.meshes[0], limits).unwrap()[..8],
            b"RLG\0\x03\0\0\0"
        );
        assert_eq!(asset.validate(limits).unwrap().adjacency_bytes, 12);
        for end in 0..bytes.len() {
            assert!(decode(&bytes[..end], limits).is_err(), "prefix {end}");
        }
        let mut corrupt = bytes.clone();
        // Header 24, mesh 4, material/vertex/index 12, tangent basis/count 8.
        corrupt[48..52].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode(&corrupt, limits).is_err());
        let mut corrupt = bytes.clone();
        let tangent_bytes = asset.meshes[0].primitives[0]
            .tangents
            .as_ref()
            .map_or(0, |t| t.values.len() * 16);
        let adjacency_start = 52 + 3 * 48 + tangent_bytes + 3 * 4;
        corrupt[adjacency_start..adjacency_start + 4].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            decode(&corrupt, limits).unwrap_err().0,
            "invalid adjacency triangle"
        );
        let mut extra = bytes;
        extra.push(0);
        assert!(decode(&extra, limits).is_err());
        let total = asset.validate(limits).unwrap().decoded_bytes;
        assert!(
            decode(
                &encode(&asset, limits).unwrap(),
                Limits {
                    max_decoded_bytes: total - 1,
                    ..limits
                }
            )
            .is_err()
        );
        let mut corrupt = asset.clone();
        corrupt.meshes[0].primitives[0].adjacency = Some(vec![0; 3]);
        assert!(encode(&corrupt, limits).is_err()); // Self reference.
        corrupt.meshes[0].primitives[0].adjacency = Some(vec![1; 3]);
        assert!(encode(&corrupt, limits).is_err()); // Out of range.
        corrupt.meshes[0].primitives[0].adjacency = Some(vec![u32::MAX; 2]);
        assert!(encode(&corrupt, limits).is_err());
    }
}

#[test]
fn adjacency_v3_can_mix_prepared_and_legacy_primitives() {
    let mut asset = triangle();
    let legacy = asset.meshes[0].primitives[0].clone();
    asset.meshes[0].primitives[0].adjacency = Some(vec![adjacency::MISSING; 3]);
    asset.meshes[0].primitives.push(legacy);
    let bytes = encode(&asset, Limits::default()).unwrap();
    assert_eq!(decode(&bytes, Limits::default()).unwrap(), asset);
}

fn mapped() -> Asset {
    let mut a = triangle();
    a.meshes[0].primitives[0].tangents = Some(TangentStream {
        basis: TangentBasis::Authored,
        values: vec![[1., 0., 0., 1.]; 3],
    });
    a.materials[0].normal_texture = Some(NormalTexture {
        texture: 0,
        sampler: Sampler::default(),
        scale: -0.5,
    });
    a.textures.push(Texture {
        role: TextureRole::NormalLinear,
        mips: vec![Mip {
            width: 1,
            height: 1,
            rgba8: vec![128, 204, 230, 255],
        }],
    });
    a
}

#[test]
fn legacy_bytes_match_golden_encoded_by_frozen_0190bf0() {
    let golden = include_bytes!("fixtures/triangle-v1.rla");
    let a = triangle();
    assert_eq!(encode(&a, Limits::default()).unwrap(), golden);
    assert_eq!(decode(golden, Limits::default()).unwrap(), a);
}

#[test]
fn v2_roundtrip_rejects_truncation_counts_basis_frames_and_nonfinite_scale() {
    let a = mapped();
    let bytes = encode(&a, Limits::default()).unwrap();
    assert_eq!(&bytes[..8], b"RLA\0\x02\0\0\0");
    assert_eq!(decode(&bytes, Limits::default()).unwrap(), a);
    for n in 0..bytes.len() {
        assert!(
            decode(&bytes[..n], Limits::unlimited()).is_err(),
            "prefix {n}"
        );
    }
    // First primitive header starts after the 24-byte asset and 4-byte mesh header.
    for (offset, word) in [
        (4, 3),
        (32, u32::MAX),
        (36, u32::MAX),
        (40, 3),
        (44, 2),
        (44, u32::MAX),
        (48 + 144 + 12, 0),
        (48 + 144, f32::NAN.to_bits()),
    ] {
        let mut bad = bytes.clone();
        bad[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        assert!(
            decode(&bad, Limits::unlimited()).is_err(),
            "offset {offset}"
        );
    }
    let mut bad = bytes.clone();
    // 48-byte primitive header + base vertices + tangent stream + indices, then material.
    let scale = 48 + 144 + 48 + 12 + 24 + 4 + 4 + 4 + 28;
    bad[scale..scale + 4].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
    assert!(decode(&bad, Limits::unlimited()).is_err());
    bad = bytes.clone();
    bad[4..8].copy_from_slice(&1u32.to_le_bytes());
    assert!(decode(&bad, Limits::unlimited()).is_err());
    bad = bytes;
    bad.push(0);
    assert!(decode(&bad, Limits::unlimited()).is_err());
}

#[test]
fn normal_references_frames_and_tangent_bytes_are_admission_boundaries() {
    let a = mapped();
    let n = a.validate(Limits::default()).unwrap().decoded_bytes;
    assert!(
        a.validate(Limits {
            max_decoded_bytes: n - 1,
            ..Limits::default()
        })
        .is_err()
    );
    let mut bad = a.clone();
    bad.meshes[0].primitives[0].tangents = None;
    assert!(bad.validate(Limits::default()).is_err());
    let mut bad = a.clone();
    bad.meshes[0].primitives[0]
        .tangents
        .as_mut()
        .unwrap()
        .values[1][3] = -1.;
    assert!(bad.validate(Limits::default()).is_err());
    let mut bad = a.clone();
    bad.textures[0].role = TextureRole::BaseColorSrgb;
    assert!(bad.validate(Limits::default()).is_err());
    let mut basic = a.clone();
    basic.materials[0].normal_texture = None;
    basic.textures.clear();
    basic.meshes[0].primitives[0].tangents = None;
    assert_eq!(
        a.validate(Limits::default()).unwrap().decoded_bytes
            - basic.validate(Limits::default()).unwrap().decoded_bytes,
        3 * 16 + 4
    );
}
