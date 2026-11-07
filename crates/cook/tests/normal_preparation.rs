use renderlab_asset::Vertex;
use renderlab_cook::{
    normal_fixture, normal_map,
    tangent::{self, PreparedTangentMesh, TangentMode, TangentSource},
};

fn vertex(position: [f32; 3], uv: [f32; 2]) -> Vertex {
    Vertex {
        position,
        normal: [0.0, 0.0, 1.0],
        uv,
        color: [0.3, 0.5, 0.7, 1.0],
    }
}
fn triangle() -> [Vertex; 3] {
    [
        vertex([0.0, 0.0, 0.0], [0.0, 0.0]),
        vertex([1.0, 0.0, 0.0], [1.0, 0.0]),
        vertex([0.0, 1.0, 0.0], [0.0, 1.0]),
    ]
}
fn near(a: [f32; 3], b: [f32; 3]) {
    for i in 0..3 {
        assert!((a[i] - b[i]).abs() < 2e-5, "{a:?} != {b:?}");
    }
}
fn correspondence(mesh: &PreparedTangentMesh, source: &[Vertex], indices: &[u32]) {
    assert_eq!(mesh.indices().len(), indices.len());
    assert_eq!(mesh.vertices().len(), mesh.tangents().len());
    for (&a, &b) in mesh.indices().iter().zip(indices) {
        assert_eq!(mesh.vertices()[a as usize], source[b as usize]);
    }
}

#[test]
fn authored_xyz_w_are_preserved_and_bitangent_uses_w() {
    let vertices = triangle();
    // Authored tangents need not agree with a newly inferred UV derivative.
    let tangents = [[0.0, 1.0, 0.0, -1.0]; 3];
    let mesh = tangent::prepare(&vertices, &[2, 0, 1], TangentSource::Authored(&tangents)).unwrap();
    assert_eq!(mesh.mode(), TangentMode::Authored);
    assert_eq!(mesh.vertices(), vertices);
    assert_eq!(mesh.indices(), [2, 0, 1]);
    assert_eq!(mesh.tangents(), tangents);
    let t = mesh.tangents()[0];
    near([t[3] * -t[1], t[3] * t[0], 0.0], [1.0, 0.0, 0.0]);
}

#[test]
fn authored_rejects_invalid_frames_counts_and_mixed_triangle_signs() {
    let vertices = triangle();
    for invalid in [
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.999],
        [2.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
        [f32::NAN, 0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0, f32::INFINITY],
    ] {
        assert!(
            tangent::prepare(
                &vertices,
                &[0, 1, 2],
                TangentSource::Authored(&[invalid; 3])
            )
            .is_err()
        );
    }
    assert!(
        tangent::prepare(
            &vertices,
            &[0, 1, 2],
            TangentSource::Authored(&[[1.0, 0.0, 0.0, 1.0]; 2])
        )
        .is_err()
    );
    let mut tangents = [[1.0, 0.0, 0.0, 1.0]; 3];
    tangents[2][3] = -1.0;
    assert!(tangent::prepare(&vertices, &[0, 1, 2], TangentSource::Authored(&tangents)).is_err());
}

#[test]
fn mikk_shared_quad_stays_indexed_and_matches_analytic_frame() {
    let mut vertices = triangle().to_vec();
    vertices.push(vertex([1.0, 1.0, 0.0], [1.0, 1.0]));
    let indices = [0, 1, 2, 2, 1, 3];
    let mesh = tangent::prepare(&vertices, &indices, TangentSource::GenerateMikkTSpace).unwrap();
    assert_eq!(mesh.mode(), TangentMode::MikkTSpace);
    assert_eq!(mesh.vertices().len(), 4);
    correspondence(&mesh, &vertices, &indices);
    for t in mesh.tangents() {
        assert_eq!(*t, [1.0, 0.0, 0.0, -1.0]);
    }
    assert_eq!(
        mesh,
        tangent::prepare(&vertices, &indices, TangentSource::GenerateMikkTSpace).unwrap()
    );
}

#[test]
fn generated_frames_match_owned_blender_gltf_plane() {
    // Owned default plane exported by Khronos glTF Blender I/O 5.1.18 with tangents.
    let vertices = [
        ([-1.0, 0.0, 1.0], [0.0, 1.0]),
        ([1.0, 0.0, 1.0], [1.0, 1.0]),
        ([-1.0, 0.0, -1.0], [0.0, 0.0]),
        ([1.0, 0.0, -1.0], [1.0, 0.0]),
    ]
    .map(|(position, uv)| Vertex {
        normal: [0.0, 1.0, 0.0],
        ..vertex(position, uv)
    });
    let indices = [0, 1, 3, 0, 3, 2];
    let exported = [[1.0, 0.0, 0.0, 1.0]; 4];
    let authored =
        tangent::prepare(&vertices, &indices, TangentSource::Authored(&exported)).unwrap();
    let generated =
        tangent::prepare(&vertices, &indices, TangentSource::GenerateMikkTSpace).unwrap();
    for (&a, &g) in authored.indices().iter().zip(generated.indices()) {
        assert_eq!(
            authored.vertices()[a as usize],
            generated.vertices()[g as usize]
        );
        assert_eq!(
            authored.tangents()[a as usize],
            generated.tangents()[g as usize]
        );
    }
}

#[test]
fn mirrored_uvs_split_shared_vertices_without_averaging_opposite_signs() {
    let mut vertices = triangle().to_vec();
    vertices.push(vertex([-1.0, 0.0, 0.0], [1.0, 0.0]));
    let indices = [0, 1, 2, 0, 2, 3];
    let mesh = tangent::prepare(&vertices, &indices, TangentSource::GenerateMikkTSpace).unwrap();
    assert_eq!(mesh.vertices().len(), 6);
    correspondence(&mesh, &vertices, &indices);
    assert_ne!(mesh.indices()[0], mesh.indices()[3]);
    assert_ne!(mesh.indices()[2], mesh.indices()[4]);
    for (face, expected) in [[1.0, 0.0, 0.0, -1.0], [-1.0, 0.0, 0.0, 1.0]]
        .iter()
        .enumerate()
    {
        for &i in &mesh.indices()[face * 3..face * 3 + 3] {
            assert_eq!(mesh.tangents()[i as usize], *expected);
        }
    }
}

#[test]
fn uv_and_normal_seams_preserve_base_attributes_and_exact_corner_domain() {
    let mut vertices = triangle().to_vec();
    vertices.extend([
        vertex([0.0, 0.0, 0.0], [1.0, 0.0]),
        vertex([0.0, 1.0, 0.0], [1.0, 1.0]),
        vertex([-1.0, 0.0, 0.0], [0.0, 0.0]),
    ]);
    let mesh = tangent::prepare(
        &vertices,
        &[0, 1, 2, 3, 4, 5],
        TangentSource::GenerateMikkTSpace,
    )
    .unwrap();
    correspondence(&mesh, &vertices, &[0, 1, 2, 3, 4, 5]);
    assert_eq!(mesh.vertices().len(), 6);
    vertices[3].normal = [1.0, 0.0, 0.0];
    vertices[4].normal = [1.0, 0.0, 0.0];
    vertices[5] = vertex([0.0, 0.0, 1.0], [0.0, 0.0]);
    vertices[5].normal = [1.0, 0.0, 0.0];
    let mesh = tangent::prepare(
        &vertices,
        &[0, 1, 2, 3, 4, 5],
        TangentSource::GenerateMikkTSpace,
    )
    .unwrap();
    correspondence(&mesh, &vertices, &[0, 1, 2, 3, 4, 5]);
    assert_eq!(mesh.vertices().len(), 6);
    assert_ne!(
        mesh.vertices()[mesh.indices()[0] as usize].normal,
        mesh.vertices()[mesh.indices()[3] as usize].normal
    );
}

#[test]
fn rejects_degenerate_uvs_and_positions_but_supports_valid_authored_uv_fallback() {
    for epsilon in [0.0, 1e-8] {
        let mut vertices = triangle();
        vertices[2].uv = [1.0, epsilon];
        assert!(
            tangent::prepare(&vertices, &[0, 1, 2], TangentSource::GenerateMikkTSpace).is_err()
        );
        assert!(
            tangent::prepare(
                &vertices,
                &[0, 1, 2],
                TangentSource::Authored(&[[1.0, 0.0, 0.0, 1.0]; 3])
            )
            .is_ok()
        );
        vertices = triangle();
        vertices[2].position = [1.0, epsilon, 0.0];
        for source in [
            TangentSource::GenerateMikkTSpace,
            TangentSource::Authored(&[[1.0, 0.0, 0.0, 1.0]; 3]),
        ] {
            assert!(tangent::prepare(&vertices, &[0, 1, 2], source).is_err());
        }
    }
    let mut vertices = triangle();
    vertices[2].uv = [0.0, 1e-8];
    assert!(tangent::prepare(&vertices, &[0, 1, 2], TangentSource::GenerateMikkTSpace).is_err());
    vertices[2].uv = [0.0, 1.0];
    for vertex in &mut vertices {
        vertex.position = vertex.position.map(|x| x * 1e-5);
    }
    assert!(tangent::prepare(&vertices, &[0, 1, 2], TangentSource::GenerateMikkTSpace).is_ok());
    let mut vertices = triangle();
    vertices[2].uv = [1.0, 1e-4];
    assert!(tangent::prepare(&vertices, &[0, 1, 2], TangentSource::GenerateMikkTSpace).is_ok());
    for v in &mut vertices {
        v.uv = v.uv.map(|x| x * 1e-25);
    }
    assert!(tangent::prepare(&vertices, &[0, 1, 2], TangentSource::GenerateMikkTSpace).is_err());
}

#[test]
fn rejects_nonfinite_attributes_nonunit_normals_and_bad_indices() {
    for indices in [&[][..], &[0, 1][..], &[0, 1, 3][..]] {
        assert!(tangent::prepare(&triangle(), indices, TangentSource::GenerateMikkTSpace).is_err());
    }
    for field in 0..4 {
        let mut vertices = triangle();
        match field {
            0 => vertices[0].position[0] = f32::INFINITY,
            1 => vertices[0].normal = [0.0, 0.0, 2.0],
            2 => vertices[0].uv[0] = f32::NAN,
            _ => vertices[0].color[0] = f32::NAN,
        }
        assert!(
            tangent::prepare(&vertices, &[0, 1, 2], TangentSource::GenerateMikkTSpace).is_err()
        );
    }
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let l = v.into_iter().map(|x| x * x).sum::<f64>().sqrt();
    v.map(|x| x / l)
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[test]
fn prepared_frames_match_nonuniform_mirrored_transform_math_reference() {
    let r = std::f32::consts::FRAC_1_SQRT_2;
    let mut vertices = triangle();
    for v in &mut vertices {
        v.normal = [r, 0.0, r];
    }
    for sign in [-1.0, 1.0] {
        let mesh = tangent::prepare(
            &vertices,
            &[0, 1, 2],
            TangentSource::Authored(&[[r, 0.0, -r, sign]; 3]),
        )
        .unwrap();
        let n = mesh.vertices()[0].normal.map(f64::from);
        let t = mesh.tangents()[0];
        // A = diag(-2,3,0.5). N uses inverse transpose; T uses A, then Gram-Schmidt.
        let n = unit([-n[0] * 0.5, n[1] / 3.0, n[2] * 2.0]);
        let at = [
            -2.0 * f64::from(t[0]),
            3.0 * f64::from(t[1]),
            0.5 * f64::from(t[2]),
        ];
        let d: f64 = n.into_iter().zip(at).map(|(n, t)| n * t).sum();
        let t_world = unit(std::array::from_fn(|i| at[i] - n[i] * d));
        let b = cross(n, t_world).map(|x| x * -f64::from(sign));
        let sqrt17 = 17.0f32.sqrt();
        near(n.map(|x| x as f32), [-1.0 / sqrt17, 0.0, 4.0 / sqrt17]);
        near(
            t_world.map(|x| x as f32),
            [-4.0 / sqrt17, 0.0, -1.0 / sqrt17],
        );
        near(b.map(|x| x as f32), [0.0, sign, 0.0]);
    }
}

#[test]
fn normal_decode_is_linear_positive_y_alpha_ignored_and_scale_is_xy_then_normalize() {
    let a = normal_map::prepare_rgba8(1, 1, &[128, 204, 230, 0]).unwrap();
    let b = normal_map::prepare_rgba8(1, 1, &[128, 204, 230, 255]).unwrap();
    assert_eq!(a, b);
    let expected = unit([1.0 / 255.0, 0.6, 205.0 / 255.0]).map(|v| v as f32);
    near(a.mips()[0].vectors()[0], expected);
    near(
        normal_map::apply_scale([0.6, 0.0, 0.8], 0.5).unwrap(),
        [0.35112345, 0.0, 0.9363292],
    );
    near(
        normal_map::apply_scale([0.6, 0.0, 0.8], -0.5).unwrap(),
        [-0.35112345, 0.0, 0.9363292],
    );
    near(
        normal_map::apply_scale([0.6, 0.0, 0.8], 0.0).unwrap(),
        [0.0, 0.0, 1.0],
    );
    assert!(normal_map::apply_scale([0.0, 0.0, 1.0], f32::INFINITY).is_err());
    assert!(normal_map::apply_scale([f32::NAN, 0.0, 1.0], 1.0).is_err());
}

#[test]
fn normal_mips_keep_mean_length_separate_from_unit_direction() {
    let map = normal_map::prepare_vectors(2, 1, vec![[0.6, 0.0, 0.8], [-0.6, 0.0, 0.8]]).unwrap();
    near(map.mips()[1].vectors()[0], [0.0, 0.0, 1.0]);
    let mixed = normal_map::prepare_vectors(2, 1, vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]).unwrap();
    near(
        mixed.mips()[1].vectors()[0],
        [
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
        ],
    );
    assert!((map.mips()[1].mean_lengths()[0] - 0.8).abs() < 1e-6);
    assert_eq!(map.mips()[1].rgba8(), [128, 128, 255, 180]);
    let uniform = normal_map::prepare_vectors(2, 1, vec![[0.0, 0.0, 1.0]; 2]).unwrap();
    assert_ne!(map.mips()[1], uniform.mips()[1]);
    assert_eq!(uniform.mips()[1].rgba8(), [128, 128, 255, 0]);
}

#[test]
fn coarse_normal_mips_average_original_means_including_cancellation() {
    // The first pair cancels; its fallback direction must never enter the next mean.
    let map = normal_map::prepare_vectors(
        4,
        1,
        vec![
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
    )
    .unwrap();
    assert_eq!(map.mips()[1].mean_lengths(), &[0.0, 1.0]);
    near(map.mips()[2].vectors()[0], [0.0, 0.0, 1.0]);
    assert!((map.mips()[2].mean_lengths()[0] - 0.5).abs() < 1e-6);
    // For asymmetric pairs, renormalizing intermediate means would also change direction.
    let map = normal_map::prepare_vectors(
        4,
        1,
        vec![
            [0.6, 0.0, 0.8],
            [-0.6, 0.0, 0.8],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
    )
    .unwrap();
    near(
        map.mips()[2].vectors()[0],
        unit([0.0, 0.5, 0.4]).map(|v| v as f32),
    );
    assert!((map.mips()[2].mean_lengths()[0] - 0.5f32.hypot(0.4)).abs() < 1e-6);
}

#[test]
fn normal_cancellation_has_declared_flat_fallback_and_odd_edges_contribute() {
    let cancelled =
        normal_map::prepare_vectors(2, 1, vec![[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]]).unwrap();
    assert_eq!(
        cancelled.mips()[1].vectors()[0],
        normal_map::ZERO_VECTOR_FALLBACK
    );
    let near_cancelled =
        normal_map::prepare_vectors(2, 1, vec![[1.0, 0.0, 1e-8], [-1.0, 0.0, 1e-8]]).unwrap();
    assert_eq!(
        near_cancelled.mips()[1].vectors()[0],
        normal_map::ZERO_VECTOR_FALLBACK
    );
    for (w, h) in [(5, 3), (3, 5)] {
        let mut vectors = vec![[0.0, 0.0, 1.0]; w * h];
        for y in 0..h {
            for x in 0..w {
                if (w == 5 && x == 4) || (h == 5 && y == 4) {
                    vectors[y * w + x] = [1.0, 0.0, 0.0];
                }
            }
        }
        let map = normal_map::prepare_vectors(w as u32, h as u32, vectors).unwrap();
        let next = map.mips()[1].vectors();
        near(next[0], [0.0, 0.0, 1.0]);
        near(next[1], unit([0.4, 0.0, 0.6]).map(|v| v as f32));
        assert_eq!((map.mips()[2].width(), map.mips()[2].height()), (1, 1));
        assert!(map.mips()[2].vectors()[0][0] > 0.0);
    }
    assert!(normal_map::prepare_rgba8(0, 0, &[]).is_err());
    assert!(normal_map::prepare_rgba8(u32::MAX, u32::MAX, &[]).is_err());
    assert!(normal_map::prepare_rgba8(1, 1, &[128, 128, 255]).is_err());
    assert!(normal_map::prepare_vectors(1, 1, vec![[f32::NAN, 0.0, 1.0]]).is_err());
}

#[test]
fn owned_gltf_fixture_roundtrips_authored_and_generated_preparation() {
    let fixture = normal_fixture::prepare().unwrap();
    assert_eq!(fixture.glb, normal_fixture::prepare().unwrap().glb);
    let gltf = gltf::Gltf::from_slice(&fixture.glb).unwrap();
    let blob = gltf.blob.as_ref().unwrap();
    let normal = gltf
        .document
        .materials()
        .next()
        .unwrap()
        .normal_texture()
        .unwrap();
    assert_eq!(normal.tex_coord(), 0);
    assert_eq!(normal.scale(), 0.5);
    for (i, mesh) in gltf.document.meshes().enumerate() {
        let primitive = mesh.primitives().next().unwrap();
        let reader = primitive.reader(|_| Some(blob.as_slice()));
        let vertices: Vec<_> = reader
            .read_positions()
            .unwrap()
            .zip(reader.read_normals().unwrap())
            .zip(reader.read_tex_coords(0).unwrap().into_f32())
            .map(|((position, normal), uv)| Vertex {
                position,
                normal,
                uv,
                color: [1.0; 4],
            })
            .collect();
        let indices: Vec<_> = reader.read_indices().unwrap().into_u32().collect();
        let tangents: Option<Vec<_>> = reader.read_tangents().map(Iterator::collect);
        let source = tangents
            .as_deref()
            .map_or(TangentSource::GenerateMikkTSpace, TangentSource::Authored);
        let prepared = tangent::prepare(&vertices, &indices, source).unwrap();
        assert_eq!(
            prepared,
            if i == 0 {
                fixture.authored.clone()
            } else {
                fixture.generated.clone()
            }
        );
    }
    let image = gltf.document.images().next().unwrap();
    let gltf::image::Source::View { view, .. } = image.source() else {
        panic!("embedded image required")
    };
    let pixels = image::load_from_memory(&blob[view.offset()..view.offset() + view.length()])
        .unwrap()
        .to_rgba8();
    assert_eq!(
        normal_map::prepare_rgba8(pixels.width(), pixels.height(), &pixels).unwrap(),
        fixture.normal_map
    );
    let n = gltf.document.nodes().nth(2).unwrap();
    assert_eq!(n.transform().decomposed().2, [-1.5, 0.75, 0.5]);
}
