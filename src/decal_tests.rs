use super::*;

#[test]
fn texture_mips_preserve_linear_premultiplied_colour() {
    let texture = DecalTexture::from_rgba8(
        DecalTextureKind::BaseColorSrgb,
        [2, 1],
        &[255, 0, 0, 255, 0, 255, 0, 0],
    )
    .unwrap();
    assert_eq!(texture.0.levels, 2);
    assert_eq!(texture.0.texels[2], [0.5, 0., 0., 0.5]);
    let odd = DecalTexture::from_rgba8(DecalTextureKind::Normal, [3, 1], &[128; 12]).unwrap();
    assert_eq!(odd.0.levels, 3);
    assert_eq!(odd.0.texels.len(), 6);
    assert!(DecalTexture::from_rgba8(DecalTextureKind::Normal, [0, 1], &[]).is_err());
}

#[test]
fn grid_never_truncates_and_preserves_stable_sort_order() {
    let decals: Vec<_> = (0..80)
        .map(|i| Decal {
            sort_order: i % 3,
            base_color: [i as f32 / 80., 0., 0., 1.],
            ..Default::default()
        })
        .collect();
    let (words, stats) = pack(
        &decals,
        DecalSettings {
            grid_resolution: [3, 1, 1],
        },
        1 << 20,
    )
    .unwrap();
    assert_eq!(stats.max_cell_decals, 80);
    assert_eq!(stats.references, 240);
    let grid = words[1] as usize;
    for cell in 0..3 {
        let first = words[grid + cell * 2] as usize;
        assert_eq!(&words[first..first + 80], &(0..80).collect::<Vec<_>>());
    }
    let mut sorted = decals.clone();
    sorted.sort_by_key(|d| d.sort_order);
    for (i, d) in sorted.iter().enumerate() {
        assert_eq!(
            f32::from_bits(words[HEADER + i * RECORD + 28]),
            d.base_color[0]
        );
    }
}

#[test]
fn invalid_decals_fail_before_publication_and_texture_uploads_are_deduplicated() {
    for transform in [
        Mat4::ZERO,
        Mat4::from_scale(Vec3::new(0., 1., 1.)),
        Mat4::perspective_rh(1., 1., 0.1, 10.),
    ] {
        assert!(
            pack(
                &[Decal {
                    transform,
                    ..Default::default()
                }],
                DecalSettings::default(),
                1 << 20
            )
            .is_err()
        );
    }
    assert!(
        pack(
            &[Decal::default()],
            DecalSettings {
                grid_resolution: [u32::MAX; 3]
            },
            1 << 20
        )
        .is_err()
    );
    assert!(pack(&[Decal::default()], DecalSettings::default(), 64).is_err());
    let texture =
        DecalTexture::from_rgba8(DecalTextureKind::BaseColorSrgb, [1, 1], &[255; 4]).unwrap();
    let decal = Decal {
        textures: DecalTextures {
            base_color: Some(texture),
            ..Default::default()
        },
        ..Default::default()
    };
    let (words, _) = pack(
        &[decal.clone(), decal],
        DecalSettings {
            grid_resolution: [1; 3],
        },
        1 << 20,
    )
    .unwrap();
    assert_eq!(words[HEADER + 44], words[HEADER + RECORD + 44]);
    assert_eq!(words[HEADER + 44] % 4, 0);
}
