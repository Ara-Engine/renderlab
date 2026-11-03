//! CPU-only actual HDR -> production decode -> prepared RLE1, with warm-cook accounting.
use renderlab_asset::environment::{self as asset, Metadata, RadianceUnits};
use renderlab_cook::{environment::*, environment_fixture, environment_hdr};
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let output = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: environment_fixture <ignored-output-directory> [--refined]")?;
    let refined = match args.next() {
        Some(flag) if flag == "--refined" => true,
        None => false,
        Some(_) => return Err("expected --refined".into()),
    };
    if args.next().is_some() {
        return Err("unexpected extra arguments".into());
    }
    let (width, height) = if refined { (1024, 512) } else { (512, 256) };
    std::fs::create_dir_all(&output)?;
    let hdr = output.join("owned-sun-sky.hdr");
    environment_fixture::write_hdr(std::fs::File::create(&hdr)?, width, height, |d| {
        environment_fixture::sun_sky(d).map(|v| v as f32)
    })?;
    let limits = CookLimits::default();
    let start = Instant::now();
    let input = environment_hdr::read_rec709_hdr(&hdr, limits)?;
    let decode_ms = start.elapsed().as_secs_f64() * 1000.0;
    let settings = if refined {
        PreparationSettings {
            diffuse_samples: 16384,
            specular_samples: 16384,
            ..PreparationSettings::default()
        }
    } else {
        PreparationSettings::default()
    };
    let cost = estimate_cost(&input, settings, limits)?;
    let metadata = Metadata {
        units: RadianceUnits::CandelasPerSquareMetre,
        source_radiance_scale: 1.0,
        rotation_y_radians: 0.0,
        generator_hash: [0; 32],
    };
    drop(prepare_environment(
        &input,
        settings,
        metadata.clone(),
        limits,
    )?);
    let start = Instant::now();
    let prepared = prepare_environment(&input, settings, metadata, limits)?;
    let cook_ms = start.elapsed().as_secs_f64() * 1000.0;
    let bytes = asset::encode(&prepared, limits.prepared)?;
    assert_eq!(asset::decode(&bytes, limits.prepared)?, prepared);
    std::fs::write(output.join("owned-sun-sky.rle"), &bytes)?;
    let input_max = input
        .pixels()
        .iter()
        .flatten()
        .copied()
        .fold(0.0_f32, f32::max);
    let mip0_max = prepared.specular_radiance()[0]
        .texels()
        .iter()
        .flatten()
        .copied()
        .fold(0.0_f32, f32::max);
    let report = format!(
        "CPU-only development-profile single warm observation; contention uncontrolled.\nsource={width}x{height} RGBE Rec709/D65, sun exponent=128 peak=2e6\nsettings={settings:?}\ndecode_ms={decode_ms:.3}\nwarm_cook_ms={cook_ms:.3}\n{cost:?}\nfile_bytes={}\ninput_max={input_max}\nmip0_max={mip0_max}\ngenerator_hash={:02x?}\n",
        bytes.len(),
        prepared.metadata().generator_hash
    );
    print!("{report}");
    std::fs::write(output.join("cost.txt"), report)?;
    Ok(())
}
