//! Measurement only: eight fixed gather rotations, without changing sky or aimed rays.
use std::sync::OnceLock;

pub(super) fn setting() -> u32 {
    static SPIN: OnceLock<u32> = OnceLock::new();
    *SPIN.get_or_init(|| {
        let value = std::env::var("RENDERLAB_GI_SPIN").unwrap_or_default();
        if value.trim().is_empty() {
            0
        } else {
            value
                .trim()
                .parse::<u32>()
                .expect("RENDERLAB_GI_SPIN must be a nonnegative integer")
                % 8
        }
    })
}

/// Each step adds 45 degrees to the cell's fixed azimuth. Only gather call sites
/// use the copy: sky visibility/update and gi_near's aimed rays keep their source.
/// Spin zero returns the original bytes, including when the variable is absent.
pub(super) fn source(mut shader: String, spin: u32) -> String {
    let spin = spin % 8;
    if spin == 0 {
        return shader;
    }
    let start = shader.find("fn gi_direction(").expect("GI direction seam");
    let end = start + shader[start..].find("\n}").expect("GI direction end") + 2;
    let original = &shader[start..end];
    let phi = "let phi=f32(k)*2.399963229728653+rotation;";
    assert_eq!(original.matches(phi).count(), 1, "GI azimuth seam");
    let direction = original.replacen("fn gi_direction(", "fn gi_spin_direction(", 1);
    let angle = f64::from(spin) * std::f64::consts::TAU / 8.0;
    let direction = direction.replacen(
        phi,
        &format!("let phi=f32(k)*2.399963229728653+rotation+{angle:.17};"),
        1,
    );
    // The following closest-hit call distinguishes gathers from all sky uses.
    let call =
        "let d=gi_direction(frame,c.key,k);\n                let h=gi_closest(origin,d,0.0,1e20);";
    let count = shader.matches(call).count();
    assert!(
        (1..=2).contains(&count),
        "GI gather direction seam: {count}"
    );
    shader = shader.replace(call, &call.replace("gi_direction(", "gi_spin_direction("));
    shader.push('\n');
    shader.push_str(&direction);
    shader
}

#[cfg(test)]
mod tests {
    use super::super::{kernel_source_base, priority};
    use super::*;

    #[test]
    fn zero_spin_preserves_modules_and_nonzero_spin_only_redirects_gathers() {
        for software in [false, true] {
            for ordered in [false, true].into_iter().filter(|&on| !on || software) {
                let original = priority::source(kernel_source_base(software, 64), ordered);
                assert_eq!(source(original.clone(), 0), original);
                assert_eq!(source(original.clone(), 8), original);
                let rotated = source(original.clone(), 1);
                let start = original.find("fn gi_direction(").unwrap();
                let end = start + original[start..].find("\n}").unwrap() + 2;
                assert!(rotated.contains(&original[start..end]));
                assert!(rotated.contains(include_str!("../shaders/gi_near.wgsl")));
                assert_eq!(
                    rotated
                        .matches("let d=gi_spin_direction(frame,c.key,k);")
                        .count(),
                    if software { 2 } else { 1 },
                );
                assert!(!rotated.contains("let d=gi_direction(frame,c.key,k);\n                let h=gi_closest(origin,d,0.0,1e20);"));
            }
        }
    }

    #[test]
    fn spin_modules_validate_on_both_paths() {
        for software in [false, true] {
            for directions in [64, 128] {
                for spin in 1..8 {
                    let shader = source(kernel_source_base(software, directions), spin);
                    let module = naga::front::wgsl::parse_str(&shader)
                        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&shader)));
                    naga::valid::Validator::new(
                        naga::valid::ValidationFlags::all(),
                        naga::valid::Capabilities::all(),
                    )
                    .validate(&module)
                    .unwrap();
                }
            }
        }
    }
}
