//! The lighting cache's work per measured frame by cause (agreed with gicut and shade, 21 Nov),
//! from a fixture's per-frame counters (their medians) and the round's pass medians:
//! - NEW (the camera): cells created, and untraced or partial cells the scan traced;
//! - MOVER: cells a moved caster invalidated (re-traced), and cells it reshaded (a light ray crossed);
//! - REFRESH: the fixed-rotation refresh's traces, and relights (lighting, sky or program changes);
//! - GATHERED: cells shaded after their gather, with the gathers and their rays.
//!
//! Every traced cell is shaded once after its trace (and again by the sky filter), so those shades
//! take the trace split. Each cause's milliseconds are estimates: `gi_update` by its share of the
//! traces, `gi_claim` to NEW, `gi_cell_shading` and `gi_integrate` (mostly the cell-space filter,
//! nine items per shaded cell) by its share of the shades, `gi_gather` to GATHERED; the probe and
//! the receivers read the cache for the screen.

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.get(sorted.len() / 2).copied().unwrap_or(0.0)
}

/// The cause line for a fixture whose series carry the cause counters, else None.
pub(super) fn line(series: &[(String, Vec<f64>)], pass_ms: impl Fn(&str) -> f64) -> Option<String> {
    let get = |name: &str| series.iter().find(|(k, _)| k == name).map(|(_, v)| median(v));
    let (created, partial, invalidated, refreshed) = (get("created")?, get("trace_partial")?, get("invalidated")?, get("refreshed")?);
    let shade = ["shade_trace", "shade_sky", "shade_relight", "shade_mover", "shade_gather"].map(|k| get(k).unwrap_or(0.0));
    let (gathers, gather_rays) = (get("gathers").unwrap_or(0.0), get("gather_rays").unwrap_or(0.0));
    let traced = created + partial + invalidated + refreshed;
    let share = |x: f64, of: f64| if of > 0.0 { x / of } else { 0.0 };
    let after = shade[0] + shade[1];
    // (traced, shaded) per cause: NEW, MOVER, REFRESH, GATHERED.
    let cells = [
        (created + partial, after * share(created + partial, traced)),
        (invalidated, after * share(invalidated, traced) + shade[3]),
        (refreshed, after * share(refreshed, traced) + shade[2]),
        (0.0, shade[4]),
    ];
    let shaded: f64 = cells.iter().map(|c| c.1).sum();
    let (update, shading) = (pass_ms("gi_update"), pass_ms("gi_cell_shading") + pass_ms("gi_integrate"));
    let mut ms = cells.map(|(t, s)| update * share(t, traced) + shading * share(s, shaded));
    ms[0] += pass_ms("gi_claim");
    ms[3] += pass_ms("gi_gather");
    let screen = pass_ms("gi_probe") + pass_ms("gi_receiver");
    let other = pass_ms("cache") - ms.iter().sum::<f64>() - screen;
    let names = ["NEW", "MOVER", "REFRESH", "GATHERED"];
    let parts: Vec<String> = names
        .iter()
        .zip(cells.iter().zip(ms))
        .map(|(name, ((t, s), ms))| {
            let extra = if *name == "GATHERED" { format!(", {gathers:.0} gathered, {gather_rays:.0} rays") } else { String::new() };
            format!("{name} {ms:.2} ms ({t:.0} traced, {s:.0} shaded{extra})")
        })
        .collect();
    Some(format!("{} | screen {screen:.2} ms (probe, receiver) | other {other:.2} ms", parts.join(" | ")))
}

#[cfg(test)]
mod tests {
    #[test]
    fn after_trace_shades_take_the_trace_split_and_passes_split_by_share() {
        let s = |k: &str, v: f64| (k.to_owned(), vec![v]);
        // 100 traced: 60 new (50 created + 10 partial), 30 mover, 10 refresh; 100 after-trace
        // shades, 200 mover reshades, 0 relights, 100 after gathers.
        let series = vec![
            s("created", 50.0), s("trace_partial", 10.0), s("invalidated", 30.0), s("refreshed", 10.0),
            s("shade_trace", 100.0), s("shade_sky", 0.0), s("shade_relight", 0.0), s("shade_mover", 200.0), s("shade_gather", 100.0),
            s("gathers", 7.0), s("gather_rays", 900.0),
        ];
        let ms = |k: &str| match k {
            "gi_update" => 1.0,
            "gi_cell_shading" => 4.0,
            "gi_gather" => 0.5,
            "cache" => 6.0,
            _ => 0.0,
        };
        let line = super::line(&series, ms).unwrap();
        // NEW: 0.6 of update + 60/400 of shading = 0.6 + 0.6; MOVER: 0.3 + 230/400 x 4 = 2.6;
        // REFRESH: 0.1 + 0.1; GATHERED: 100/400 x 4 + 0.5 = 1.5; other 6 - 5.5 = 0.5.
        assert!(line.starts_with("NEW 1.20 ms (60 traced, 60 shaded) | MOVER 2.60 ms (30 traced, 230 shaded) | REFRESH 0.20 ms (10 traced, 10 shaded)"), "{line}");
        assert!(line.contains("GATHERED 1.50 ms (0 traced, 100 shaded, 7 gathered, 900 rays)"), "{line}");
        assert!(line.ends_with("other 0.50 ms"), "{line}");
    }
}
