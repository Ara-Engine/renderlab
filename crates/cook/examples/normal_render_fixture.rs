//! Owned normal-bearing GLB -> regular v2 -> virtual v2 inputs for the GPU phase.
use renderlab_asset::{self as asset, virtual_mesh::VirtualLimits};
use renderlab_cook::{
    self as cook, CookLimits, Settings, normal_fixture, virtual_cache, virtual_cook,
};
use serde_json::json;
use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("expected output directory")?;
    fs::create_dir_all(&directory)?;
    normal_fixture::generate(&directory)?;
    let source = directory.join("normal-mapped.glb");
    let cache = directory.join("cache");
    let result = cook::cook_file(
        &source,
        &cache,
        &directory.join("normal.manifest"),
        Settings {
            normal_maps: true,
            ..Default::default()
        },
        CookLimits::default(),
    )?;
    let original = result.loaded.asset;
    let mut records = vec![];
    for name in ["mapped", "flat", "unmapped", "negative-scale", "zero-scale"] {
        let mut a = original.clone();
        match name {
            "flat" => {
                for texture in &mut a.textures {
                    if texture.role == asset::TextureRole::NormalLinear {
                        for mip in &mut texture.mips {
                            for pixel in mip.rgba8.chunks_exact_mut(4) {
                                pixel.copy_from_slice(&[128, 128, 255, 255]);
                            }
                        }
                    }
                }
            }
            "unmapped" => {
                for material in &mut a.materials {
                    material.normal_texture = None;
                }
            }
            "negative-scale" => {
                for material in &mut a.materials {
                    if let Some(t) = &mut material.normal_texture {
                        t.scale = -1.;
                    }
                }
            }
            "zero-scale" => {
                for material in &mut a.materials {
                    if let Some(t) = &mut material.normal_texture {
                        t.scale = 0.;
                    }
                }
            }
            _ => {}
        }
        let rla = asset::encode(&a, asset::Limits::default())?;
        let payload = directory.join(format!("{name}.rla"));
        fs::write(&payload, &rla)?;
        let virtual_result = virtual_cache::cook_file(
            &payload,
            &cache,
            &directory.join(format!("{name}.virtual")),
            virtual_cook::Settings::default(),
            VirtualLimits::default(),
        )?;
        let counts = a.validate(asset::Limits::default())?;
        records.push(json!({"case":name,"payload_sha256":cook::hash_hex(&rla),"virtual_sha256":virtual_result.loaded.package_sha256,"tangent_vertices":counts.tangent_vertices,"tangent_bytes":counts.tangent_bytes,"virtual_payload_bytes":virtual_result.loaded.counts.payload_bytes,"virtual_disk_bytes":virtual_result.loaded.counts.disk_page_bytes}));
    }
    fs::write(
        directory.join("render-inputs.json"),
        serde_json::to_vec_pretty(
            &json!({"source_sha256":cook::hash_hex(&fs::read(source)?),"recipe":"normal-v2/linear+Y/mikktspace-0.3.0/normalized-area-mips-v1/locked-tangent-seams","limits":"default","cases":records,"quality":"GPU verification pending; no image or Unreal parity claim"}),
        )?,
    )?;
    Ok(())
}
