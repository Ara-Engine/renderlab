//! Experimental fog alternatives share one process-level setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Default,
    Coherent,
    Map,
    Quarter,
}

pub(crate) fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(
        || match std::env::var("RENDERLAB_ATMOSPHERE_PASS").as_deref() {
            Ok("coherent") => Mode::Coherent,
            Ok("map") => Mode::Map,
            Ok("quarter") => Mode::Quarter,
            Ok(_) => Mode::Default,
            Err(_) if std::env::var("RENDERLAB_ATMOSPHERE_TILE").as_deref() == Ok("8x4") => {
                Mode::Coherent
            }
            Err(_) => Mode::Default,
        },
    )
}
