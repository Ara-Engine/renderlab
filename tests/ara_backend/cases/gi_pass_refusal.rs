//! `RENDERLAB_GI_PASS=1` on the port: the GI lookup pass is wired for wgpu only, so
//! `Engine::new` refuses the switch with a message instead of composing a read of binding 68
//! that nothing allocates or writes. Runs with the switch set (read once per process);
//! without it the test says so and passes.
use super::*;
use renderlab::{backend_ara::engine::Engine, engine::Settings};

#[test]
fn the_port_refuses_the_gi_lookup_pass_switch() -> Result<()> {
    if std::env::var("RENDERLAB_GI_PASS").as_deref() != Ok("1") {
        println!("gi pass refusal: RENDERLAB_GI_PASS=1 not set, nothing checked");
        return Ok(());
    }
    let path = root("gi-pass-refusal");
    let refused = Engine::new(ShaderCache::new(path.join("shaders"))?, Settings::default());
    let message = refused
        .err()
        .context("the port started with RENDERLAB_GI_PASS=1")?
        .to_string();
    println!("gi pass refusal: {message}");
    ensure!(
        message.contains("RENDERLAB_GI_PASS=1"),
        "the refusal names the switch: {message}"
    );
    Ok(())
}

/// `RENDERLAB_LIGHT_VIS_PASS=1` on the port: the local-light visibility pass is wired for wgpu
/// only, so `Engine::new` refuses it with a message instead of composing a read of binding 73
/// that nothing allocates or writes. Runs with the switch set (read once per process); without
/// it the test says so and passes.
#[test]
fn the_port_refuses_the_light_visibility_pass_switch() -> Result<()> {
    if std::env::var("RENDERLAB_LIGHT_VIS_PASS").as_deref() != Ok("1")
        || std::env::var_os("RENDERLAB_LIGHT_BOUND").is_some()
    {
        println!(
            "light visibility pass refusal: RENDERLAB_LIGHT_VIS_PASS=1 not set, nothing checked"
        );
        return Ok(());
    }
    let path = root("light-vis-pass-refusal");
    let refused = Engine::new(ShaderCache::new(path.join("shaders"))?, Settings::default());
    let message = refused
        .err()
        .context("the port started with RENDERLAB_LIGHT_VIS_PASS=1")?
        .to_string();
    println!("light visibility pass refusal: {message}");
    ensure!(
        message.contains("RENDERLAB_LIGHT_VIS_PASS=1"),
        "the refusal names the switch: {message}"
    );
    Ok(())
}
