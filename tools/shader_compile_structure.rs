//! CPU-only pipeline census. Run the ignored test with SHADER_STRUCTURE_OUT set.
//! No Instance, Adapter or Device is created. See docs/shader-compile-structure.md.
use super::*;
use naga::back::spv;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};

#[path = "shader_compile_sources.rs"]
mod catalogue;

#[derive(Default)]
struct Function {
    id: u32,
    words: usize,
    instructions: usize,
    calls: Vec<u32>,
    blocks: BTreeMap<u32, Vec<u32>>,
    loops: Vec<(u32, u32)>,
    opcodes: Vec<u16>,
}
struct Stats {
    instructions: usize,
    functions: Vec<Function>,
    names: BTreeMap<u32, String>,
}

fn string(words: &[u32]) -> Result<String, String> {
    let bytes: Vec<_> = words
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .take_while(|&b| b != 0)
        .collect();
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// Counts encoded instructions, not dynamic execution. Loop depth uses CFG regions
/// bounded by each OpLoopMerge's merge target, including its continue construct.
fn inspect(words: &[u32]) -> Result<Stats, String> {
    if words.len() < 5 || words[0] != 0x07230203 {
        return Err("bad SPIR-V header".into());
    }
    let mut result = Stats {
        instructions: 0,
        functions: vec![],
        names: BTreeMap::new(),
    };
    let mut function: Option<Function> = None;
    let mut block = 0;
    let mut widths = BTreeMap::new();
    let mut types = BTreeMap::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        let op = (words[at] & 0xffff) as u16;
        if count == 0 || at + count > words.len() {
            return Err(format!("truncated instruction at {at}"));
        }
        let args = &words[at + 1..at + count];
        result.instructions += 1;
        if op == 5 && args.len() >= 2 {
            result.names.insert(args[0], string(&args[1..])?);
        }
        if op == 15 && args.len() >= 3 {
            result.names.insert(args[1], string(&args[2..])?);
        }
        if op == 21 && args.len() >= 2 {
            widths.insert(args[0], args[1]);
        }
        // Result-type/result-ID prefix of values that may feed OpSwitch.
        if matches!(
            op,
            43 | 44 | 45 | 46 | 48 | 49 | 50 | 52 | 55 | 57 | 61 | 83 | 124 | 128..=200 | 245
        ) && args.len() >= 2
        {
            types.insert(args[1], args[0]);
        }
        if op == 54 {
            if function.is_some() || args.len() != 4 {
                return Err("bad OpFunction".into());
            }
            function = Some(Function {
                id: args[1],
                ..Default::default()
            });
        }
        if let Some(f) = &mut function {
            f.words += count;
            f.instructions += 1;
            // DEBUG attribution is compared against these non-debug opcode streams.
            if !matches!(op, 8 | 317) {
                f.opcodes.push(op);
            }
            match op {
                57 if args.len() >= 3 => f.calls.push(args[2]),
                248 if args.len() == 1 => {
                    block = args[0];
                    f.blocks.entry(block).or_default();
                }
                246 if args.len() >= 2 => f.loops.push((block, args[0])),
                249 if args.len() == 1 => {
                    f.blocks.entry(block).or_default().push(args[0]);
                }
                250 if args.len() >= 3 => {
                    f.blocks
                        .entry(block)
                        .or_default()
                        .extend_from_slice(&args[1..3]);
                }
                251 if args.len() >= 2 => {
                    let edges = f.blocks.entry(block).or_default();
                    edges.push(args[1]);
                    let wide = types
                        .get(&args[0])
                        .and_then(|t| widths.get(t))
                        .copied()
                        .unwrap_or(32)
                        > 32;
                    for case in args[2..].chunks(if wide { 3 } else { 2 }) {
                        if let Some(&target) = case.last() {
                            edges.push(target);
                        }
                    }
                }
                _ => {}
            }
        }
        if op == 56 {
            result
                .functions
                .push(function.take().ok_or("OpFunctionEnd outside function")?);
        }
        at += count;
    }
    if function.is_some() {
        return Err("unterminated function".into());
    }
    Ok(result)
}

fn depth(function: &Function) -> usize {
    let mut membership = BTreeMap::<u32, usize>::new();
    for &(header, merge) in &function.loops {
        let mut seen = BTreeSet::new();
        let mut work = vec![header];
        while let Some(block) = work.pop() {
            if block == merge || !seen.insert(block) {
                continue;
            }
            *membership.entry(block).or_default() += 1;
            if let Some(edges) = function.blocks.get(&block) {
                work.extend(edges);
            }
        }
    }
    membership.values().copied().max().unwrap_or(0)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// The same runtime-check switches as Vulkan compile_stage. Device-specific
/// robustness/workgroup initialization are an explicit synthetic reference profile.
fn options(module: &naga::Module, checked: bool, debug: bool) -> spv::Options<'static> {
    let robust = std::env::var("SHADER_STRUCTURE_ROBUST").as_deref() != Ok("0");
    let native_zero = std::env::var("SHADER_STRUCTURE_NATIVE_ZERO").as_deref() != Ok("0");
    let mut binding_map = spv::BindingMap::default();
    for (_, global) in module.global_variables.iter() {
        if let Some(binding) = &global.binding {
            let size = match module.types[global.ty].inner {
                naga::TypeInner::BindingArray {
                    size: naga::ArraySize::Constant(n),
                    ..
                } => Some(n.get()),
                _ => None,
            };
            binding_map.insert(
                binding.clone(),
                spv::BindingInfo {
                    descriptor_set: binding.group,
                    binding: binding.binding,
                    binding_array_size: size,
                },
            );
        }
    }
    let mut flags = spv::WriterFlags::LABEL_VARYINGS | spv::WriterFlags::FORCE_POINT_SIZE;
    flags.set(spv::WriterFlags::DEBUG, debug);
    use naga::proc::BoundsCheckPolicy::{Restrict, Unchecked};
    spv::Options {
        lang_version: (1, 6),
        flags,
        fake_missing_bindings: false,
        binding_map,
        bounds_check_policies: naga::proc::BoundsCheckPolicies {
            index: if checked { Restrict } else { Unchecked },
            buffer: if checked && !robust {
                Restrict
            } else {
                Unchecked
            },
            image_load: if checked && !robust {
                Restrict
            } else {
                Unchecked
            },
            binding_array: Unchecked,
        },
        zero_initialize_workgroup_memory: if native_zero {
            spv::ZeroInitializeWorkgroupMemoryMode::Native
        } else {
            spv::ZeroInitializeWorkgroupMemoryMode::Polyfill
        },
        force_loop_bounding: checked,
        ray_query_initialization_tracking: checked,
        emit_int_div_checks: checked,
        use_storage_input_output_16: false,
        ..Default::default()
    }
}

/// DEBUG is only a name-attribution sidecar. Verify function ordering and opcode
/// streams match before attaching its names to the measured no-debug binary.
fn metrics(words: &[u32], named: &[u32]) -> Result<Value, String> {
    let plain = inspect(words)?;
    let named = inspect(named)?;
    if plain.functions.len() != named.functions.len() {
        return Err("DEBUG changed function inventory".into());
    }
    let mut names = plain.names;
    for (a, b) in plain.functions.iter().zip(&named.functions) {
        if a.opcodes != b.opcodes {
            return Err("DEBUG changed function opcode order".into());
        }
        names.entry(a.id).or_insert_with(|| {
            named
                .names
                .get(&b.id)
                .cloned()
                .unwrap_or_else(|| format!("generated_{}", a.id))
        });
    }
    let mut calls = BTreeMap::<u32, usize>::new();
    for f in &plain.functions {
        for &target in &f.calls {
            *calls.entry(target).or_default() += 1;
        }
    }
    let mut functions: Vec<_> = plain.functions.iter().map(|f| json!({
        "name": names.get(&f.id), "id": f.id, "bytes": f.words * 4,
        "instructions": f.instructions, "loops": f.loops.len(), "max_loop_nesting": depth(f),
        "call_sites": calls.get(&f.id).copied().unwrap_or(0),
        "callees": f.calls.iter().map(|id| names.get(id).cloned().unwrap_or_else(|| format!("id_{id}"))).collect::<Vec<_>>()
    })).collect();
    functions.sort_by_key(|f| std::cmp::Reverse(f["bytes"].as_u64().unwrap()));
    Ok(
        json!({"spirv_bytes": words.len() * 4, "instructions": plain.instructions,
        "function_count": functions.len(),
        "loop_count": plain.functions.iter().map(|f| f.loops.len()).sum::<usize>(),
        "max_loop_nesting": plain.functions.iter().map(depth).max().unwrap_or(0),
        "top_functions": functions.iter().take(10).collect::<Vec<_>>(),
        "large_helpers": functions.iter().filter(|f| f["instructions"].as_u64().unwrap() >= 100).collect::<Vec<_>>(),
        "functions": functions}),
    )
}

#[test]
fn cpu_shader_structure_analyzer() {
    let source = "fn helper(x:u32)->u32{return x+1u;} @compute @workgroup_size(1) fn main(){var n=0u; loop {if n>3u{break;} var m=0u; loop {if m>n{break;} n=helper(n); m+=1u;} n+=1u;}}";
    let module = naga::front::wgsl::parse_str(source).unwrap();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let pipeline = spv::PipelineOptions {
        shader_stage: naga::ShaderStage::Compute,
        entry_point: "main".into(),
    };
    let plain = spv::write_vec(
        &module,
        &info,
        &options(&module, false, false),
        Some(&pipeline),
    )
    .unwrap();
    let named = spv::write_vec(
        &module,
        &info,
        &options(&module, false, true),
        Some(&pipeline),
    )
    .unwrap();
    let stats = metrics(&plain, &named).unwrap();
    assert_eq!(stats["loop_count"], 2);
    assert_eq!(stats["max_loop_nesting"], 2);
    assert_eq!(
        stats["functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "helper")
            .unwrap()["call_sites"],
        1
    );
    assert!(inspect(&[0x07230203, 0, 0, 0, 0, 0]).is_err());
}

#[test]
#[ignore = "CPU-only shader census; emits potentially large ignored WGSL/SPIR-V dumps"]
fn cpu_shader_compile_structure() {
    let out = std::env::var("SHADER_STRUCTURE_OUT")
        .expect("set SHADER_STRUCTURE_OUT to an ignored output directory");
    let out = Path::new(&out);
    fs::create_dir_all(out).unwrap();
    let glass = crate::STANDARD_PROGRAM.replace(
        "return (1.0-s.alpha)*(1.0-fresnel)*s.base;",
        "return (1.0-s.alpha)*(1.0-fresnel)*s.base*vec3<f32>(0.8,1.0,0.6);",
    );
    assert_ne!(
        glass,
        crate::STANDARD_PROGRAM,
        "glass transmittance edit seam changed"
    );
    let programs = [
        ("standard", crate::STANDARD_PROGRAM),
        ("toon", include_str!("../shaders/programs/toon.wgsl")),
        ("glass_tint", glass.as_str()),
    ];
    let optimizer = std::env::var("SHADER_STRUCTURE_SPIRV_OPT").ok();
    let mut optimized = BTreeMap::<String, Value>::new();
    let mut records = vec![];
    let mut failures = vec![];
    for (variant, program) in programs {
        let sources = catalogue::sources(program).expect("production source catalogue");
        for source in sources {
            let safe: String = source
                .name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let stem = format!("{variant}-{}-{safe}", source.path);
            fs::write(out.join(format!("{stem}.wgsl")), &source.source).unwrap();
            let module = match naga::front::wgsl::parse_str(&source.source) {
                Ok(m) => m,
                Err(e) => {
                    failures.push(json!({"variant":variant,"module":source.name,"phase":"parse","error":e.emit_to_string(&source.source)}));
                    continue;
                }
            };
            let info = match naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            {
                Ok(i) => i,
                Err(e) => {
                    failures.push(json!({"variant":variant,"module":source.name,"phase":"validate","error":e.emit_to_string(&source.source)}));
                    continue;
                }
            };
            for entry in &source.entry_points {
                let Some(ep) = module.entry_points.iter().find(|e| &e.name == entry) else {
                    failures.push(json!({"variant":variant,"module":source.name,"entry":entry,"phase":"catalogue","error":"selected production entry missing"}));
                    continue;
                };
                let constants = source.constants.iter().cloned().collect();
                let processed = naga::back::pipeline_constants::process_overrides(
                    &module,
                    &info,
                    Some((ep.stage, entry)),
                    &constants,
                );
                let (ir, ir_info) = match processed {
                    Ok(x) => x,
                    Err(e) => {
                        failures.push(json!({"variant":variant,"module":source.name,"entry":entry,"phase":"overrides","error":e.to_string()}));
                        continue;
                    }
                };
                for checked in [true, false] {
                    let mode = if checked { "checked" } else { "unchecked" };
                    let name = format!("{stem}-{entry}-{mode}");
                    let pipeline = spv::PipelineOptions {
                        shader_stage: ep.stage,
                        entry_point: entry.to_string(),
                    };
                    let result = (|| -> Result<Value, String> {
                        let words = spv::write_vec(
                            &ir,
                            &ir_info,
                            &options(&ir, checked, false),
                            Some(&pipeline),
                        )
                        .map_err(|e| e.to_string())?;
                        let named = spv::write_vec(
                            &ir,
                            &ir_info,
                            &options(&ir, checked, true),
                            Some(&pipeline),
                        )
                        .map_err(|e| e.to_string())?;
                        let mut data = metrics(&words, &named)?;
                        let raw = bytes(&words);
                        let hash = digest(&raw);
                        let spv_path = out.join(format!("{name}.spv"));
                        fs::write(&spv_path, raw).map_err(|e| e.to_string())?;
                        if let Some(tool) = &optimizer {
                            if !optimized.contains_key(&hash) {
                                let path = out.join(format!("{name}-O.spv"));
                                let run = Command::new(tool)
                                    .args(["-O", "--target-env=vulkan1.3"])
                                    .arg(&spv_path)
                                    .arg("-o")
                                    .arg(&path)
                                    .output()
                                    .map_err(|e| e.to_string())?;
                                let value = if run.status.success() {
                                    json!({"bytes": fs::metadata(path).map_err(|e|e.to_string())?.len()})
                                } else {
                                    // Keep strict validation failure visible. A separate size-only
                                    // diagnostic may still show optimizer potential; never label
                                    // its output validated or use it in production.
                                    let diagnostic = Command::new(tool)
                                        .args(["-O", "--target-env=vulkan1.3", "--skip-validation"])
                                        .arg(&spv_path)
                                        .arg("-o")
                                        .arg(&path)
                                        .output()
                                        .map_err(|e| e.to_string())?;
                                    let unchecked_size = if diagnostic.status.success() {
                                        json!({"bytes": fs::metadata(&path).map_err(|e|e.to_string())?.len()})
                                    } else {
                                        json!({"error":String::from_utf8_lossy(&diagnostic.stderr)})
                                    };
                                    json!({"error":String::from_utf8_lossy(&run.stderr),
                                        "skip_validation_diagnostic":unchecked_size})
                                };
                                optimized.insert(hash.clone(), value);
                            }
                            data["optimized"] = optimized[&hash].clone();
                        }
                        data["sha256"] = json!(hash);
                        Ok(data)
                    })();
                    match result {
                        Ok(mut data) => {
                            data["variant"] = json!(variant); data["module"] = json!(source.name);
                            data["path"] = json!(source.path); data["entry"] = json!(entry); data["stage"] = json!(format!("{:?}", ep.stage));
                            data["checks"] = json!(mode); data["engine_owned"] = json!(source.engine_owned);
                            data["wgsl_lines"] = json!(source.source.lines().count());
                            data["wgsl_sha256"] = json!(digest(source.source.as_bytes()));
                            data["ir_functions_before_entry_processing"] = json!(module.functions.len());
                            data["ir_functions_after_entry_processing"] = json!(ir.functions.len());
                            data["constants"] = json!(source.constants);
                            records.push(data);
                        }
                        Err(e) => failures.push(json!({"variant":variant,"module":source.name,"entry":entry,"checks":mode,"phase":"emit_or_analyze","error":e})),
                    }
                }
            }
            eprintln!(
                "census variant={variant} module={} records={} failures={}",
                source.name,
                records.len(),
                failures.len()
            );
            // Leave incremental evidence if a later module exposes a backend failure.
            fs::write(
                out.join("results.json"),
                serde_json::to_vec_pretty(&json!({"records":records,"failures":failures})).unwrap(),
            )
            .unwrap();
        }
    }
    records.sort_by_key(|r| std::cmp::Reverse(r["spirv_bytes"].as_u64().unwrap()));
    let optimizer_version = optimizer
        .as_ref()
        .and_then(|p| Command::new(p).arg("--version").output().ok())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let report = json!({"schema":1,"naga":"30.0.1","spirv_version":"1.6", "profile":{
        "robust_buffer_and_image_access":std::env::var("SHADER_STRUCTURE_ROBUST").as_deref()!=Ok("0"),
        "native_workgroup_zero_init":std::env::var("SHADER_STRUCTURE_NATIVE_ZERO").as_deref()!=Ok("0"),
        "binding_map":"explicit identity descriptor numbers and WGSL fixed binding-array sizes",
        "capabilities":"unrestricted structural diagnostic; no device capability certification", "debug":false,
        "zero_initialize_workgroup_memory":"enabled independently of runtime checks"},
        "optimizer":optimizer_version,"records":records,"failures":failures});
    fs::write(
        out.join("results.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    eprintln!(
        "census complete records={} failures={} out={}",
        report["records"].as_array().unwrap().len(),
        report["failures"].as_array().unwrap().len(),
        out.display()
    );
    assert!(
        report["failures"].as_array().unwrap().is_empty(),
        "see results.json for explicit census failures"
    );
}
