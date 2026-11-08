//! CPU-only GLB -> shared RLA preparation for the Ara/Renderlab measurement harness.
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("ara1_prepare SOURCE.glb OUTPUT.rla".into());
    }
    let source = fs::read(&args[0])?;
    let limits = renderlab_cook::CookLimits::unlimited();
    let asset = renderlab_cook::prepare(
        &source,
        renderlab_cook::Settings {
            normal_maps: true,
            ..Default::default()
        },
        limits,
    )?;
    let output = PathBuf::from(&args[1]);
    fs::create_dir_all(output.parent().ok_or("output has no parent")?)?;
    fs::write(&output, renderlab::asset::encode(&asset, limits.prepared)?)?;
    eprintln!(
        "{}: {} meshes, {} placements",
        output.display(),
        asset.meshes.len(),
        asset.placements.len()
    );
    Ok(())
}
