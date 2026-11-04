//! Explicit offline field preparation. `probe` runs the production host/kernel before `full` admission.
use renderlab_asset::environment::{self as asset, CubeFace, Metadata, RadianceUnits};
use renderlab_cook::{
    environment::{gpu::*, *},
    environment_hdr,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::{Duration, Instant},
};
#[path = "../../../support/validation.rs"]
mod validation;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn number(
    v: &serde_json::Value,
    key: &str,
    default: u64,
) -> Result<u64, Box<dyn std::error::Error>> {
    v.get(key).map_or(Ok(default), |x| {
        x.as_u64()
            .ok_or_else(|| format!("policy {key} must be an unsigned integer").into())
    })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _validation =
        validation::begin(std::env::var("RENDERLAB_REQUIRE_VALIDATION").as_deref() == Ok("1"));
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 4 || !matches!(args[0].as_str(), "probe" | "full") {
        return Err("usage: environment_field probe INPUT.hdr ORACLE.json REPORT.json [POLICY.json] | full INPUT.hdr EARLY_REPORT.json OUTPUT.rle2 [POLICY.json]".into());
    }
    if args.len() > 5 {
        return Err("unexpected arguments".into());
    }
    let policy: serde_json::Value = if let Some(p) = args.get(4) {
        serde_json::from_slice(&fs::read(p)?)?
    } else {
        serde_json::json!({})
    };
    let allowed = [
        "max_sample_evaluations",
        "max_working_bytes",
        "max_source_file_bytes",
        "max_source_texels",
        "max_prepared_bytes",
        "max_dimension",
        "wait_seconds",
        "tile_milliseconds",
        "wall_seconds",
    ];
    if policy
        .as_object()
        .ok_or("policy must be an object")?
        .keys()
        .any(|k| !allowed.contains(&k.as_str()))
    {
        return Err("unknown policy field".into());
    }
    // Full is an explicitly selected expensive profile; every provided caller policy overrides it.
    let defaults = CookLimits::default();
    let limits = CookLimits {
        max_sample_evaluations: number(
            &policy,
            "max_sample_evaluations",
            if args[0] == "full" {
                104_000_000_000
            } else {
                defaults.max_sample_evaluations
            },
        )?,
        max_working_bytes: number(&policy, "max_working_bytes", defaults.max_working_bytes)?,
        max_source_file_bytes: number(
            &policy,
            "max_source_file_bytes",
            defaults.max_source_file_bytes,
        )?,
        max_source_texels: number(&policy, "max_source_texels", defaults.max_source_texels)?,
        prepared: asset::Limits {
            max_payload_bytes: number(&policy, "max_prepared_bytes", 128 * 1024 * 1024)?,
            max_dimension: u32::try_from(number(
                &policy,
                "max_dimension",
                u64::from(defaults.prepared.max_dimension),
            )?)?,
            ..defaults.prepared
        },
        ..defaults
    };
    let gpu_policy = GpuPolicy {
        wait_timeout: Duration::from_secs(number(&policy, "wait_seconds", 30)?),
        max_tile_time: Duration::from_millis(number(&policy, "tile_milliseconds", 1000)?),
        max_wall_time: Duration::from_secs(number(
            &policy,
            "wall_seconds",
            if args[0] == "full" { 600 } else { 120 },
        )?),
    };
    let total = Instant::now();
    let source_hash = file_digest(Path::new(&args[1]), limits.max_source_file_bytes)?;
    let tool_hash = digest(&fs::read(std::env::current_exe()?)?);
    let decode = Instant::now();
    let input = environment_hdr::read_rec709_hdr(Path::new(&args[1]), limits)?;
    let decode_ms = decode.elapsed().as_secs_f64() * 1000.0;
    let identity = serde_json::json!({"source_sha256":source_hash,"tool_sha256":tool_hash,
        "recipe":String::from_utf8_lossy(RECIPE),"kernel_sha256":digest(KERNEL.as_bytes())});
    if args[0] == "probe" {
        let oracle: serde_json::Value = serde_json::from_slice(&fs::read(&args[2])?)?;
        let rows = oracle
            .as_array()
            .ok_or("oracle must be an array of keyed texel references")?;
        if rows.is_empty() || rows.len() > 6 * 128 * 128 {
            return Err("oracle count".into());
        }
        let mut keys = Vec::with_capacity(rows.len());
        for row in rows {
            let k = row["key"].as_array().ok_or("oracle key")?;
            if k.len() != 4 {
                return Err("oracle key length".into());
            }
            let values: Vec<_> = k
                .iter()
                .map(|x| x.as_u64().ok_or("oracle key integer"))
                .collect::<Result<_, _>>()?;
            keys.push(ConvolutionTexel {
                level: values[0].try_into()?,
                face: *CubeFace::ALL.get(values[1] as usize).ok_or("face")?,
                y: values[2].try_into()?,
                x: values[3].try_into()?,
            });
        }
        let mut gpu = GpuPreparer::new(&input, limits, gpu_policy)?;
        let actual = gpu.prepare_texels(&keys, 1.0)?;
        let mut max_ratio = 0.0_f64;
        let mut checked = 0;
        let mut independent = 0;
        for (row, value) in rows.iter().zip(&actual) {
            for field in ["f64", "reference_rgb"] {
                if let Some(refs) = row.get(field) {
                    let refs = refs.as_array().ok_or("reference RGB")?;
                    if refs.len() != 3 {
                        return Err("reference RGB length".into());
                    }
                    if field == "reference_rgb" {
                        if row["converged"].as_bool() != Some(true) {
                            return Err("unconverged independent reference".into());
                        }
                        independent += 1;
                    }
                    for c in 0..3 {
                        let reference = refs[c].as_f64().ok_or("finite reference")?;
                        if reference < 0.0 {
                            return Err("negative reference".into());
                        }
                        let ratio =
                            (f64::from(value[c]) - reference).abs() / (0.04 * reference + 0.001);
                        max_ratio = max_ratio.max(ratio);
                        if ratio > 1.0 {
                            return Err(format!("precision gate failed key={} channel={c} actual={} reference={reference} ratio={ratio}", row["key"], value[c]).into());
                        }
                    }
                    checked += 1;
                }
            }
        }
        if checked < rows.len() {
            return Err("missing numerical reference".into());
        }
        let report = serde_json::json!({"identity":identity,"passed":true,"texels":rows.len(),"independent_texels":independent,
            "max_allowance_ratio":max_ratio,"actual_rgb":actual,"timing":format!("{:?}",gpu.report),"decode_ms":decode_ms});
        publish(Path::new(&args[3]), &serde_json::to_vec_pretty(&report)?)?;
        println!("{report}");
        return Ok(());
    }
    let gate: serde_json::Value = serde_json::from_slice(&fs::read(&args[2])?)?;
    if gate["identity"] != identity
        || gate["passed"].as_bool() != Some(true)
        || gate["texels"].as_u64().unwrap_or(0) < 10
        || gate["independent_texels"].as_u64().unwrap_or(0) < 10
    {
        return Err("full field requires this exact binary/source/recipe's passing ten-texel independent early gate".into());
    }
    if Path::new(&args[3]).exists() {
        return Err("output exists; choose a new RLE2 cache path".into());
    }
    let cost = estimate_field_cost(&input, limits)?;
    eprintln!("full field explicit admission {cost:?}; policy={policy}");
    let (prepared, report) = prepare_environment_gpu(
        &input,
        Metadata {
            units: RadianceUnits::CandelasPerSquareMetre,
            rotation_y_radians: 0.0,
            source_radiance_scale: 1.0,
            generator_hash: [0; 32],
        },
        limits,
        gpu_policy,
    )?;
    let at = Instant::now();
    let bytes = asset::encode(&prepared, limits.prepared)?;
    let encode_ms = at.elapsed().as_secs_f64() * 1000.0;
    let at = Instant::now();
    publish(Path::new(&args[3]), &bytes)?;
    let write_ms = at.elapsed().as_secs_f64() * 1000.0;
    let artifact_hash = digest(&bytes);
    drop(bytes);
    drop(prepared);
    let at = Instant::now();
    let loaded = fs::read(&args[3])?;
    let cached = asset::decode(&loaded, limits.prepared)?;
    let cache_load_ms = at.elapsed().as_secs_f64() * 1000.0;
    let result = serde_json::json!({"identity":identity,"artifact_sha256":artifact_hash,
        "generator":format!("{:02x?}",cached.metadata().generator_hash),"prepared_bytes":cached.pixel_bytes(),
        "decode_ms":decode_ms,"encode_ms":encode_ms,"write_ms":write_ms,"cache_load_ms":cache_load_ms,
        "end_to_end_ms":total.elapsed().as_secs_f64()*1000.0,"cook":format!("{report:?}"),
        "cost":format!("{cost:?}"),"limits":format!("{limits:?}"),"gpu_policy":format!("{gpu_policy:?}")});
    let report_path = format!("{}.json", args[3]);
    publish(
        Path::new(&report_path),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{result}");
    Ok(())
}

fn publish(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension(format!("partial-{}", std::process::id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::hard_link(&temporary, path)
    })();
    fs::remove_file(temporary)?;
    result
}

fn file_digest(path: &Path, cap: u64) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    if file.metadata()?.len() > cap {
        return Err(std::io::Error::other("source file byte policy"));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    let mut read = 0_u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        read += n as u64;
        if read > cap {
            return Err(std::io::Error::other("source file grew beyond policy"));
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
