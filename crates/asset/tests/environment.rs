use renderlab_asset::environment::*;

fn metadata() -> Metadata {
    Metadata {
        units: RadianceUnits::CandelasPerSquareMetre,
        rotation_y_radians: 0.75,
        source_radiance_scale: 2.0,
        generator_hash: [19; 32],
    }
}
fn prepared() -> PreparedEnvironment {
    let limits = Limits::default();
    PreparedEnvironment::new(
        metadata(),
        CubeLevel::new(1, vec![[1e-20, 2.0, 1e30]; 6], limits).unwrap(),
        vec![
            CubeLevel::new(2, vec![[0.0, 1e6, 1e-20]; 24], limits).unwrap(),
            CubeLevel::new(1, vec![[3.0; 3]; 6], limits).unwrap(),
        ],
        limits,
    )
    .unwrap()
}

#[test]
fn deterministic_roundtrip_keeps_f32_range_and_metadata() {
    let p = prepared();
    let bytes = encode(&p, Limits::default()).unwrap();
    assert_eq!(bytes.len(), HEADER_BYTES + p.pixel_bytes() as usize);
    assert_eq!(&bytes[..4], b"RLE1");
    let decoded = decode(&bytes, Limits::default()).unwrap();
    assert_eq!(p, decoded);
    assert_eq!(bytes, encode(&decoded, Limits::default()).unwrap());
    assert!(decoded.specular_radiance()[0].texels()[0][1] > 65504.0);
}

#[test]
fn strict_file_boundary_rejects_truncation_corruption_and_trailing_bytes() {
    let bytes = encode(&prepared(), Limits::default()).unwrap();
    for len in 0..bytes.len() {
        assert!(
            decode(&bytes[..len], Limits::default()).is_err(),
            "truncated at {len}"
        );
    }
    for (offset, value) in [
        (0, 0u32),
        (4, VERSION + 1),
        (8, 2),
        (12, 2),
        (16, 0),
        (20, 2),
        (24, f32::NAN.to_bits()),
        (28, 0),
        (64, 0),
        (68, 3),
        (72, 99),
        (76, 1),
        (80, 0),
        (88, u32::MAX),
        (HEADER_BYTES, f32::INFINITY.to_bits()),
        (HEADER_BYTES + 4, (-1.0_f32).to_bits()),
    ] {
        let mut bad = bytes.clone();
        bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(
            decode(&bad, Limits::unbounded()).is_err(),
            "corrupt at {offset}"
        );
    }
    let mut extra = bytes;
    extra.push(0);
    assert!(decode(&extra, Limits::default()).is_err());
}

#[test]
fn configurable_budgets_and_structural_bounds_are_independent() {
    let p = prepared();
    let bytes = encode(&p, Limits::default()).unwrap();
    for limits in [
        Limits {
            max_dimension: 1,
            ..Limits::unbounded()
        },
        Limits {
            max_texels: p.texel_count() - 1,
            ..Limits::unbounded()
        },
        Limits {
            max_payload_bytes: p.pixel_bytes() - 1,
            ..Limits::unbounded()
        },
    ] {
        assert!(decode(&bytes, limits).is_err());
        assert!(encode(&p, limits).is_err());
    }
    assert!(decode(&bytes, Limits::unbounded()).is_ok());
    assert!(cube_texels(u32::MAX, Limits::unbounded()).is_err());
    assert!(layout_texels(1, 0, Limits::unbounded()).is_err());
    assert!(layout_texels(1, 3, Limits::unbounded()).is_err());
    assert!(layout_texels(1, 1 << 31, Limits::unbounded()).is_err());
    assert!(CubeLevel::new(0, vec![], Limits::unbounded()).is_err());
    assert!(CubeLevel::new(1, vec![[0.0; 3]; 5], Limits::default()).is_err());
    assert!(CubeLevel::new(1, vec![[-1.0; 3]; 6], Limits::default()).is_err());
    assert!(
        PreparedEnvironment::new(
            metadata(),
            CubeLevel::new(1, vec![[0.0; 3]; 6], Limits::default()).unwrap(),
            vec![],
            Limits::default()
        )
        .is_err()
    );
}

#[test]
fn all_twelve_cube_edges_have_identical_source_directions() {
    use CubeFace::*;
    assert_eq!(PositiveX.direction([0.0, 0.0]), [1.0, 0.0, 0.0]);
    assert_eq!(NegativeX.direction([0.0, 0.0]), [-1.0, 0.0, 0.0]);
    assert_eq!(PositiveY.direction([0.0, 0.0]), [0.0, 1.0, 0.0]);
    assert_eq!(NegativeY.direction([0.0, 0.0]), [0.0, -1.0, 0.0]);
    assert_eq!(PositiveZ.direction([0.0, 0.0]), [0.0, 0.0, 1.0]);
    assert_eq!(NegativeZ.direction([0.0, 0.0]), [0.0, 0.0, -1.0]);
    for t in [-1.0, -0.6, 0.0, 0.3, 1.0] {
        for (a, uv, b, st) in [
            (PositiveX, [-1.0, t], PositiveZ, [1.0, t]),
            (PositiveX, [1.0, t], NegativeZ, [-1.0, t]),
            (NegativeX, [-1.0, t], NegativeZ, [1.0, t]),
            (NegativeX, [1.0, t], PositiveZ, [-1.0, t]),
            (PositiveX, [t, -1.0], PositiveY, [1.0, -t]),
            (PositiveX, [t, 1.0], NegativeY, [1.0, t]),
            (NegativeX, [t, -1.0], PositiveY, [-1.0, t]),
            (NegativeX, [t, 1.0], NegativeY, [-1.0, -t]),
            (PositiveZ, [t, -1.0], PositiveY, [t, 1.0]),
            (PositiveZ, [t, 1.0], NegativeY, [t, -1.0]),
            (NegativeZ, [t, -1.0], PositiveY, [-t, -1.0]),
            (NegativeZ, [t, 1.0], NegativeY, [-t, 1.0]),
        ] {
            assert_eq!(a.direction(uv), b.direction(st), "{a:?} {b:?}");
        }
    }
}
