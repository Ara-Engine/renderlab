//! `cargo run -p renderlab-cook --example skin_asset -- input.glb cache-dir`
use renderlab_cook::{CookLimits, Settings, skin};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let source = args.get(1).ok_or("source GLB required")?;
    let cache = args.get(2).ok_or("cache directory required")?;
    let cooked = skin::cook_file(
        source.as_ref(),
        cache.as_ref(),
        Settings {
            normal_maps: true,
            ..Default::default()
        },
        CookLimits::default(),
    )?;
    let asset = renderlab_asset::skin::decode(
        &std::fs::read(&cooked.path)?,
        renderlab_asset::Limits::default(),
    )?;
    println!(
        "path={} cache_hit={} meshes={} bindings={} joints={} clips={}",
        cooked.path.display(),
        cooked.cache_hit,
        asset.geometry.meshes.len(),
        asset.bindings.len(),
        asset.skins.iter().map(|s| s.joints.len()).sum::<usize>(),
        asset.clips.len()
    );
    Ok(())
}
