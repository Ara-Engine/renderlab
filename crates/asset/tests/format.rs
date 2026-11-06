use renderlab_asset::*;

fn fixture() -> Asset {
    let vertices = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        .map(|position| Vertex {
            position,
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
            color: [1.0; 4],
        })
        .to_vec();
    Asset {
        meshes: vec![Mesh {
            primitives: vec![Primitive {
                adjacency: None,
                tangents: None,
                vertices,
                indices: vec![0, 1, 2],
                material: 0,
            }],
        }],
        placements: vec![Placement {
            mesh: 0,
            transform: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }],
        materials: vec![Material {
            base_color_texture: Some(BaseColorTexture {
                texture: 0,
                sampler: Sampler::default(),
            }),
            ..Material::default()
        }],
        textures: vec![Texture {
            role: TextureRole::BaseColorSrgb,
            mips: vec![Mip {
                width: 1,
                height: 1,
                rgba8: vec![128, 64, 32, 255],
            }],
        }],
    }
}

#[test]
fn deterministic_round_trip_rejects_every_truncation_and_trailing_data() {
    let a = fixture();
    let limits = Limits::default();
    let bytes = encode(&a, limits).unwrap();
    assert_eq!(decode(&bytes, limits).unwrap(), a);
    assert_eq!(bytes, encode(&a, limits).unwrap());
    assert_eq!(&bytes[..8], b"RLA\0\x01\0\0\0");
    for length in 0..bytes.len() {
        assert!(decode(&bytes[..length], limits).is_err(), "prefix {length}");
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(decode(&extra, limits).is_err());
    let mut corrupt = bytes.clone();
    corrupt[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode(&corrupt, limits).is_err());
    let mut corrupt = bytes.clone();
    corrupt[40..44].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
    assert!(decode(&corrupt, limits).is_err());
    let mut corrupt = bytes.clone();
    let end = corrupt.len();
    corrupt[end - 16..end - 12].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        decode(&corrupt, limits).unwrap_err().0,
        "texture dimension limit"
    );
    let mut corrupt = bytes.clone();
    corrupt[end - 8..end - 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode(&corrupt, limits).is_err());
    // Header 24, mesh header 4, primitive header 12, then three 48-byte vertices.
    let mut corrupt = bytes;
    corrupt[184..188].copy_from_slice(&99u32.to_le_bytes());
    assert!(decode(&corrupt, limits).is_err());
}

#[test]
fn scalar_reference_transform_and_mip_contracts_are_checked() {
    let limits = Limits::default();
    let a = fixture();
    let mut bad = a.clone();
    bad.meshes[0].primitives[0].material = 9;
    assert!(encode(&bad, limits).is_err());
    let mut bad = a.clone();
    bad.placements[0].mesh = 1;
    assert!(encode(&bad, limits).is_err());
    for value in [f32::NAN, f32::INFINITY] {
        let mut bad = a.clone();
        bad.placements[0].transform[0][0] = value;
        assert!(encode(&bad, limits).is_err());
    }
    let mut bad = a.clone();
    bad.placements[0].transform[0][0] = 0.0;
    assert!(encode(&bad, limits).is_err());
    let mut bad = a.clone();
    bad.placements[0].transform[0][3] = 0.5;
    assert!(encode(&bad, limits).is_err());
    let mut mirrored = a.clone();
    mirrored.placements[0].transform[0][0] = -2.0;
    assert!(encode(&mirrored, limits).is_ok());
    let mut bad = a.clone();
    bad.textures[0].role = TextureRole::MetallicRoughnessLinear;
    assert!(encode(&bad, limits).is_err());
    let mut bad = a.clone();
    bad.textures[0].mips[0].width = 0;
    assert!(encode(&bad, limits).is_err());
    let mut bad = a.clone();
    bad.textures[0].mips[0].rgba8.pop();
    assert!(encode(&bad, limits).is_err());
    let mut bad = a.clone();
    let mip = bad.textures[0].mips[0].clone();
    bad.textures[0].mips.push(mip);
    assert!(encode(&bad, limits).is_err());
    let mut bad = a.clone();
    bad.textures[0].mips[0] = Mip {
        width: 2,
        height: 1,
        rgba8: vec![0; 8],
    };
    assert!(encode(&bad, limits).is_err());
    let bytes = encode(&a, limits).unwrap();
    for limits in [
        Limits {
            max_vertices: 2,
            ..limits
        },
        Limits {
            max_decoded_bytes: 1,
            ..limits
        },
        Limits {
            max_file_bytes: bytes.len() - 1,
            ..limits
        },
        Limits {
            max_dimension: 0,
            ..limits
        },
    ] {
        assert!(decode(&bytes, limits).is_err());
    }
}

#[test]
fn uncapped_policy_keeps_truncation_overflow_and_validity_checks() {
    let a = fixture();
    let limits = Limits::unlimited();
    let bytes = encode(&a, limits).unwrap();
    assert_eq!(decode(&bytes, limits).unwrap(), a);
    for n in 0..bytes.len() {
        assert!(decode(&bytes[..n], limits).is_err());
    }
    let mut bad = bytes.clone();
    bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode(&bad, limits).is_err());
    let mut invalid = a;
    invalid.meshes[0].primitives[0].indices[0] = u32::MAX;
    assert!(invalid.validate(limits).is_err());
}
