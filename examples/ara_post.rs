//! A stored linear HDR frame becomes a PNG and a Mac window entirely on Ara's GPU.
#[cfg(target_os = "macos")]
#[path = "ara_post/app.rs"]
mod app;
#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    app::run()
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!(
        "ara_post window currently requires macOS; portable pass replays run in tests/ara_backend.rs"
    );
}
