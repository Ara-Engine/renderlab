//! Recook the owned analytic HDR at explicit spatial/sample quality, without a new source copy.
use renderlab_asset::environment::{self as asset, Metadata, RadianceUnits};
use renderlab_cook::{environment::*, environment_hdr};
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 4 {
        return Err(
            "usage: environment_quality INPUT.hdr OUTPUT.rle DIFFUSE_SIZE SPECULAR_SIZE".into(),
        );
    }
    let settings = PreparationSettings {
        diffuse_size: args[2].parse()?,
        specular_size: args[3].parse()?,
        diffuse_samples: 16384,
        specular_samples: 16384,
    };
    let limits = CookLimits {
        max_sample_evaluations: 250_000_000,
        ..Default::default()
    };
    let input = environment_hdr::read_rec709_hdr(PathBuf::from(&args[0]).as_path(), limits)?;
    let cost = estimate_cost(&input, settings, limits)?;
    eprintln!("admitted settings={settings:?} cost={cost:?}");
    let start = Instant::now();
    let prepared = prepare_environment(
        &input,
        settings,
        Metadata {
            units: RadianceUnits::CandelasPerSquareMetre,
            source_radiance_scale: 1.,
            rotation_y_radians: 0.,
            generator_hash: [0; 32],
        },
        limits,
    )?;
    let cook_ms = start.elapsed().as_secs_f64() * 1e3;
    let bytes = asset::encode(&prepared, limits.prepared)?;
    std::fs::write(&args[1], &bytes)?;
    println!(
        "cook_ms={cook_ms:.3} bytes={} generator={:02x?}",
        bytes.len(),
        prepared.metadata().generator_hash
    );
    Ok(())
}
