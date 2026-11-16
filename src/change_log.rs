//! Measurement only (sunrt, 2026-11-16): `RENDERLAB_CHANGE_LOG=<file>` writes one JSON line per
//! report naming which inputs of the renderer's own change tests differ from the same site's
//! previous report: what makes a played frame (characters, spinners, metering, the player rig)
//! cost more than the clip's. Unset, each site pays one cached environment read.
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::sync::{Mutex, OnceLock};

struct Log {
    out: std::io::BufWriter<std::fs::File>,
    previous: HashMap<&'static str, Vec<u64>>,
}

fn log() -> Option<&'static Mutex<Log>> {
    static LOG: OnceLock<Option<Mutex<Log>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let file = std::fs::File::create(std::env::var_os("RENDERLAB_CHANGE_LOG")?).ok()?;
        Some(Mutex::new(Log {
            out: std::io::BufWriter::new(file),
            previous: HashMap::new(),
        }))
    })
    .as_ref()
}

/// Whether the log is on; sites skip building their inputs when it is not.
pub(crate) fn enabled() -> bool {
    log().is_some()
}

/// Reports `site` for renderer frame `frame`: `parts` are named inputs, hashed and compared with
/// the site's previous report (`changed` lists those that differ), and `values` are written as
/// given (counts, flags).
pub(crate) fn report(frame: u64, site: &'static str, parts: &[(&str, &[u8])], values: &[(&str, String)]) {
    let Some(log) = log() else {
        return;
    };
    let Ok(mut log) = log.lock() else {
        return;
    };
    let hashes: Vec<u64> = parts
        .iter()
        .map(|(_, part)| {
            let mut h = DefaultHasher::new();
            part.hash(&mut h);
            h.finish()
        })
        .collect();
    let changed: Vec<&str> = match log.previous.get(site) {
        Some(old) if old.len() == hashes.len() => parts
            .iter()
            .zip(old.iter().zip(&hashes))
            .filter(|(_, (a, b))| a != b)
            .map(|((name, _), _)| *name)
            .collect(),
        _ => vec!["first"],
    };
    log.previous.insert(site, hashes);
    let mut line = format!("{{\"frame\":{frame},\"site\":\"{site}\",\"changed\":[");
    line.push_str(&changed.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(","));
    line.push(']');
    for (name, value) in values {
        line.push_str(&format!(",\"{name}\":{value}"));
    }
    line.push_str("}\n");
    let _ = log.out.write_all(line.as_bytes()).and_then(|()| log.out.flush());
}
