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

fn field() -> PreparedEnvironment {
    let limits = Limits::default();
    PreparedEnvironment::new_independent(
        metadata(),
        CubeLevel::new(1, vec![[1e-20, 2.0, 1e30]; 6], limits).unwrap(),
        (0..FIELD_LEVELS)
            .map(|i| CubeLevel::new(2, vec![[i as f32, 1e6, 1e-20]; 24], limits).unwrap())
            .collect(),
        roughness_knots(),
        FieldGenerator::F64HostF32GpuV1,
        limits,
    )
    .unwrap()
}

#[test]
fn independent_roundtrip_and_exact_roughness_brackets() {
    let p = field();
    let bytes = encode(&p, Limits::default()).unwrap();
    assert_eq!(&bytes[..4], b"RLE2");
    assert_eq!(bytes.len(), RLE2_HEADER_BYTES + p.pixel_bytes() as usize);
    assert_eq!(p, decode(&bytes, Limits::default()).unwrap());
    assert_eq!(
        bytes,
        encode(
            &decode(&bytes, Limits::default()).unwrap(),
            Limits::default()
        )
        .unwrap()
    );
    assert!(matches!(
        prepared().specular_layout(),
        SpecularLayout::LegacyMipChain
    ));
    let knots = roughness_knots();
    for i in 0..64 {
        let (lo, hi, t) = roughness_bracket(knots[i]).unwrap();
        assert_eq!((lo, hi, t), (i, i + 1, 0.0));
        let r = (knots[i] + knots[i + 1]) * 0.5;
        let (lo, hi, t) = roughness_bracket(r).unwrap();
        assert_eq!((lo, hi), (i, i + 1));
        let recovered = (1.0 - t) * knots[lo].powi(4) + t * knots[hi].powi(4);
        assert!((recovered - r.powi(4)).abs() < 1e-6);
    }
    assert_eq!(roughness_bracket(1.0).unwrap(), (64, 64, 0.0));
    for r in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(roughness_bracket(r).is_err());
    }
    assert_eq!(
        field_texels(16, 128, Limits::default()).unwrap() * 12,
        76_695_552
    );
}

#[test]
fn independent_rejects_conventions_knots_payload_dimensions_and_policy() {
    let p = field();
    let bytes = encode(&p, Limits::default()).unwrap();
    for (at, v) in [
        (4, 1_u32),
        (8, 2),
        (12, 2),
        (20, 1),
        (64, 0),
        (68, 3),
        (72, 64),
        (76, 1),
        (80, u32::MAX),
        (88, u32::MAX),
        (96, 2),
        (100, 2),
        (104, 2),
        (108, 2),
        (112, 64),
        (116, 1),
        (120, 0),
        (128, (-0.0_f32).to_bits()),
        (132, f32::NAN.to_bits()),
        (136, 1.0_f32.to_bits()),
        (RLE2_HEADER_BYTES, f32::INFINITY.to_bits()),
        (bytes.len() - 4, (-1.0_f32).to_bits()),
    ] {
        let mut bad = bytes.clone();
        bad[at..at + 4].copy_from_slice(&v.to_le_bytes());
        assert!(decode(&bad, Limits::unbounded()).is_err(), "offset {at}");
    }
    for len in [0, 95, 96, 127, 128, RLE2_HEADER_BYTES - 1, bytes.len() - 1] {
        assert!(decode(&bytes[..len], Limits::default()).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(decode(&trailing, Limits::default()).is_err());
    for limits in [
        Limits {
            max_dimension: 1,
            ..Limits::default()
        },
        Limits {
            max_texels: p.texel_count() - 1,
            ..Limits::default()
        },
        Limits {
            max_payload_bytes: p.pixel_bytes() - 1,
            ..Limits::default()
        },
    ] {
        assert!(decode(&bytes, limits).is_err());
        assert!(encode(&p, limits).is_err());
    }
    let mut legacy = encode(&prepared(), Limits::default()).unwrap();
    legacy[..4].copy_from_slice(b"RLE2");
    assert!(decode(&legacy, Limits::default()).is_err());
    let mut new = bytes;
    new[..4].copy_from_slice(b"RLE1");
    assert!(decode(&new, Limits::default()).is_err());
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
