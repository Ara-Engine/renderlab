//! Animated GLB recipe and its own content-addressed `skin-v1` cache namespace.
use crate::{CookLimits, Error, Result, Settings, ensure};
use renderlab_asset::skin::*;
use std::path::{Path, PathBuf};

pub fn prepare(source: &[u8], settings: Settings, limits: CookLimits) -> Result<SkinnedAsset> {
    let geometry = crate::import::prepare_inner(source, settings, limits, true)?;
    let gltf = gltf::Gltf::from_slice(source).map_err(|e| Error(e.to_string()))?;
    let blob = gltf
        .blob
        .as_deref()
        .ok_or_else(|| Error("skin GLB requires BIN".into()))?;
    let document = &gltf.document;
    ensure(
        document
            .nodes()
            .all(|n| matches!(n.transform(), gltf::scene::Transform::Decomposed { .. })),
        "skin-v1 requires TRS nodes; matrix nodes must be converted to TRS before cooking",
    )?;
    let mut parents = vec![None; document.nodes().len()];
    for node in document.nodes() {
        for child in node.children() {
            parents[child.index()] = Some(node.index() as u32);
        }
    }
    let nodes = document
        .nodes()
        .map(|n| {
            let (translation, rotation, scale) = n.transform().decomposed();
            Node {
                parent: parents[n.index()],
                translation,
                rotation,
                scale,
            }
        })
        .collect();
    let identity = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];
    for skin in document.skins() {
        if let Some(a) = skin.inverse_bind_matrices() {
            ensure(
                a.data_type() == gltf::accessor::DataType::F32
                    && a.dimensions() == gltf::accessor::Dimensions::Mat4
                    && !a.normalized()
                    && a.count() == skin.joints().len(),
                "inverse bind accessor must be float MAT4, one per joint",
            )?;
        }
    }
    let skins = document
        .skins()
        .map(|s| {
            let joints: Vec<_> = s.joints().map(|n| n.index() as u32).collect();
            let inverse_bind = s
                .reader(|_| Some(blob))
                .read_inverse_bind_matrices()
                .map(Iterator::collect)
                .unwrap_or_else(|| vec![identity; joints.len()]);
            Skin {
                joints,
                inverse_bind,
            }
        })
        .collect();
    // Keep only bindings from the selected scene; geometry's placement order is unchanged.
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or_else(|| Error("skin scene missing".into()))?;
    let mut stack: Vec<_> = scene.nodes().collect();
    stack.reverse();
    let mut bindings = vec![];
    while let Some(n) = stack.pop() {
        if let (Some(mesh), Some(skin)) = (n.mesh(), n.skin()) {
            bindings.push(Binding {
                mesh: mesh.index() as u32,
                node: n.index() as u32,
                skin: skin.index() as u32,
            });
        }
        let children: Vec<_> = n.children().collect();
        stack.extend(children.into_iter().rev());
    }
    let mut influences = vec![];
    for (m, cooked) in document.meshes().zip(&geometry.meshes) {
        let mut streams = vec![];
        for (p, cooked) in m.primitives().zip(&cooked.primitives) {
            let reader = p.reader(|_| Some(blob));
            let joints = reader
                .read_joints(0)
                .map(|j| j.into_u16().collect::<Vec<_>>());
            let weights = reader
                .read_weights(0)
                .map(|w| w.into_f32().collect::<Vec<_>>());
            let stream = match (joints, weights) {
                (None, None) => None,
                (Some(joints), Some(weights)) => {
                    ensure(joints.len() == weights.len(), "joint/weight count mismatch")?;
                    let mut source = vec![];
                    for (joints, mut weights) in joints.into_iter().zip(weights) {
                        let sum: f32 = weights.iter().sum();
                        ensure(
                            weights.iter().all(|w| w.is_finite() && *w >= 0.)
                                && sum.is_finite()
                                && sum > 0.,
                            "invalid skin weights",
                        )?;
                        for w in &mut weights {
                            *w /= sum;
                        }
                        source.push(Influence { joints, weights });
                    }
                    let indices: Vec<_> = reader
                        .read_indices()
                        .map(|i| i.into_u32().collect())
                        .unwrap_or_else(|| (0..source.len() as u32).collect());
                    ensure(
                        indices.len() == cooked.indices.len(),
                        "skin tangent topology changed",
                    )?;
                    // Flat normals and tangent generation may duplicate vertices. Preserve
                    // influences through the common indexed-corner domain, never by position.
                    let mut remapped = vec![None; cooked.vertices.len()];
                    for (&old, &new) in indices.iter().zip(&cooked.indices) {
                        let value = source
                            .get(old as usize)
                            .ok_or_else(|| Error("skin source index".into()))?;
                        let dst = &mut remapped[new as usize];
                        ensure(
                            dst.as_ref().is_none_or(|v| v == value),
                            "skin remap merged distinct influences",
                        )?;
                        *dst = Some(value.clone());
                    }
                    // Unreferenced vertices keep their original attributes when no remap occurred.
                    Some(
                        remapped
                            .into_iter()
                            .enumerate()
                            .map(|(i, v)| {
                                v.or_else(|| {
                                    (source.len() == cooked.vertices.len())
                                        .then(|| source[i].clone())
                                })
                                .ok_or_else(|| Error("unmapped skin vertex".into()))
                            })
                            .collect::<Result<Vec<_>>>()?,
                    )
                }
                _ => return Err(Error("JOINTS_0 and WEIGHTS_0 must appear together".into())),
            };
            streams.push(stream);
        }
        influences.push(streams);
    }
    let mut clips = vec![];
    for a in document.animations() {
        let mut channels = vec![];
        for c in a.channels() {
            use gltf::animation::util::ReadOutputs;
            use gltf::animation::{Interpolation as I, Property as P};
            let reader = c.reader(|_| Some(blob));
            let input = c.sampler().input();
            let output = c.sampler().output();
            ensure(
                input.data_type() == gltf::accessor::DataType::F32
                    && input.dimensions() == gltf::accessor::Dimensions::Scalar
                    && !input.normalized(),
                "animation input must be float SCALAR",
            )?;
            ensure(
                output.data_type() == gltf::accessor::DataType::F32
                    && !output.normalized()
                    && output.dimensions()
                        == if c.target().property() == P::Rotation {
                            gltf::accessor::Dimensions::Vec4
                        } else {
                            gltf::accessor::Dimensions::Vec3
                        },
                "animation output must be float VEC3/VEC4 for the target",
            )?;
            let property = match c.target().property() {
                P::Translation => Property::Translation,
                P::Rotation => Property::Rotation,
                P::Scale => Property::Scale,
                P::MorphTargetWeights => {
                    return Err(Error("morph animation is unsupported".into()));
                }
            };
            let interpolation = match c.sampler().interpolation() {
                I::Step => Interpolation::Step,
                I::Linear => Interpolation::Linear,
                I::CubicSpline => Interpolation::CubicSpline,
            };
            let times = reader
                .read_inputs()
                .ok_or_else(|| Error("animation inputs".into()))?
                .collect();
            let values = match reader
                .read_outputs()
                .ok_or_else(|| Error("animation outputs".into()))?
            {
                ReadOutputs::Translations(v) | ReadOutputs::Scales(v) => {
                    v.map(|v| [v[0], v[1], v[2], 0.]).collect()
                }
                ReadOutputs::Rotations(v) => v.into_f32().collect(),
                _ => return Err(Error("animation output type".into())),
            };
            channels.push(Channel {
                node: c.target().node().index() as u32,
                property,
                interpolation,
                times,
                values,
            });
        }
        clips.push(Clip { channels });
    }
    let asset = SkinnedAsset {
        geometry,
        nodes,
        skins,
        bindings,
        influences,
        clips,
    };
    asset
        .validate(limits.prepared)
        .map_err(|e| Error(e.to_string()))?;
    Ok(asset)
}

pub struct CookedSkin {
    pub path: PathBuf,
    pub cache_hit: bool,
}
/// The source, recipe settings and limits participate in the key. Existing static
/// cache manifests are neither read nor overwritten by this recipe.
pub fn cook_file(
    source: &Path,
    cache: &Path,
    settings: Settings,
    limits: CookLimits,
) -> Result<CookedSkin> {
    let bytes = crate::cache::read_bounded(source, limits.source_bytes)?;
    let mut recipe = format!("renderlab/skin-v1/gltf-1.4.1/{settings:?}/{limits:?}").into_bytes();
    recipe.extend(&bytes);
    let key = crate::cache::hash_hex(&recipe);
    let path = cache.join("skin-v1").join(format!("{key}.rls"));
    if crate::cache::read_bounded(&path, limits.prepared.max_file_bytes)
        .ok()
        .is_some_and(|b| renderlab_asset::skin::decode(&b, limits.prepared).is_ok())
    {
        return Ok(CookedSkin {
            path,
            cache_hit: true,
        });
    }
    let asset = prepare(&bytes, settings, limits)?;
    let encoded =
        renderlab_asset::skin::encode(&asset, limits.prepared).map_err(|e| Error(e.to_string()))?;
    crate::cache::immutable_entry(&path, &encoded)?;
    Ok(CookedSkin {
        path,
        cache_hit: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut bin = vec![];
        for x in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            bin.extend(x.to_le_bytes());
        }
        for _ in 0..3 {
            bin.extend([0, 0, 0, 0]);
        }
        for _ in 0..3 {
            for x in [1f32, 0., 0., 0.] {
                bin.extend(x.to_le_bytes());
            }
        }
        for x in [0f32, 1.] {
            bin.extend(x.to_le_bytes());
        }
        for x in [0f32, 0., 0., 2., 0., 0.] {
            bin.extend(x.to_le_bytes());
        }
        let json = serde_json::json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0,1]}],
            "nodes":[{"mesh":0,"skin":0},{}],"skins":[{"joints":[1]}],
            "buffers":[{"byteLength":bin.len()}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":12},{"buffer":0,"byteOffset":48,"byteLength":48},{"buffer":0,"byteOffset":96,"byteLength":8},{"buffer":0,"byteOffset":104,"byteLength":24}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},{"bufferView":1,"componentType":5121,"count":3,"type":"VEC4"},{"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"},{"bufferView":3,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]},{"bufferView":4,"componentType":5126,"count":2,"type":"VEC3"}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2}}]}],
            "animations":[{"samplers":[{"input":3,"output":4}],"channels":[{"sampler":0,"target":{"node":1,"path":"translation"}}]}]});
        let mut json = serde_json::to_vec(&json).unwrap();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend(((28 + json.len() + bin.len()) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(0x4e4f534au32.to_le_bytes());
        out.extend(json);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(0x004e4942u32.to_le_bytes());
        out.extend(bin);
        out
    }
    #[test]
    fn skin_recipe_roundtrips_and_legacy_static_recipe_still_rejects_animation() {
        let source = fixture();
        let limits = CookLimits::default();
        assert!(crate::prepare(&source, Settings::default(), limits).is_err());
        let asset = prepare(&source, Settings::default(), limits).unwrap();
        assert_eq!(asset.bindings.len(), 1);
        assert_eq!(asset.clips[0].duration(), 1.);
        assert_eq!(asset.influences[0][0].as_ref().unwrap().len(), 3);
        let bytes = renderlab_asset::skin::encode(&asset, limits.prepared).unwrap();
        assert_eq!(
            renderlab_asset::skin::decode(&bytes, limits.prepared).unwrap(),
            asset
        );
        assert!(renderlab_asset::decode(&bytes, limits.prepared).is_err());
        assert!(renderlab_asset::skin::decode(&bytes[..bytes.len() - 1], limits.prepared).is_err());
        let mut invalid = asset.clone();
        invalid.skins[0].joints[0] = 999;
        assert!(invalid.validate(limits.prepared).is_err());
        invalid = asset.clone();
        invalid.nodes[0].parent = Some(1);
        invalid.nodes[1].parent = Some(0);
        assert!(invalid.validate(limits.prepared).is_err());
        invalid = asset;
        invalid.clips[0].channels[0].times[1] = 0.;
        assert!(invalid.validate(limits.prepared).is_err());
    }
}
