//! A program's reflection, taken from one parse and kept beside the cooked code.
//!
//! Reflecting a program means parsing its whole WGSL with naga and selecting its entry:
//! about 0.2 s of the first frame of every process on the Mac for the game scene, and
//! most of a second on the Radeon, though the cooked code itself is on disk. The inputs
//! and the workgroup size now come from one parse, kept for the process by the pass and
//! the source's digest, and written beside the cooked code (`<cache root>/reflection/`)
//! so a later process reads them back instead of parsing. A stored record is keyed by
//! the shader cache's salt (the adapter's sources, the compiler, `Cargo.lock`), the pass
//! and the digest; one that fails to read is reflected again and rewritten.
use super::{InputKey, InputKind, Pass, SampleType, ShaderInput};
use anyhow::{Context, Result, ensure};
use ara_gpu::{descriptors::Format, texture::ViewDimension};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

/// What reflection gives a program: its packet inputs, and a compute entry's
/// workgroup size (`None` for raster passes). Passes outside the packet family (the
/// post chain, the light clusters) may hold resources the packet reflection does not
/// take; they are asked only for their workgroup size, so their inputs keep the error
/// for whoever asks.
#[derive(Clone)]
pub(super) struct Reflected {
    pub inputs: std::result::Result<Vec<ShaderInput>, String>,
    pub threads: Option<[u32; 3]>,
}
impl Reflected {
    /// The packet inputs, or the reason the pass has none.
    pub fn inputs(self) -> Result<Vec<ShaderInput>> {
        self.inputs.map_err(anyhow::Error::msg)
    }
}

/// Where stored reflections go, and the salt their keys take: set by
/// `ShaderCache::new`. Unset, reflection is kept for the process only.
static STORE: OnceLock<Mutex<Option<Store>>> = OnceLock::new();
/// The store's directory and the salt its keys take.
type Store = (PathBuf, [u8; 32]);

/// Stores reflections under `root` (the shader cache's directory), keyed by `salt`.
pub(crate) fn store_in(root: PathBuf, salt: [u8; 32]) {
    *STORE
        .get_or_init(Default::default)
        .lock()
        .expect("reflection store lock") = Some((root.join("reflection"), salt));
}

/// `source`'s reflection for `pass`, whose digest is `digest`.
pub(super) fn reflected(pass: Pass, source: &str, digest: [u8; 32]) -> Result<Reflected> {
    static KEPT: super::memo::Memo<(Pass, [u8; 32]), Reflected> = super::memo::Memo::new();
    KEPT.get((pass, digest), || {
        let path = STORE
            .get_or_init(Default::default)
            .lock()
            .expect("reflection store lock")
            .as_ref()
            .map(|(dir, salt)| {
                let mut key = Sha256::new();
                key.update(salt);
                key.update(format!("{pass:?}"));
                key.update(digest);
                dir.join(format!("{:x}.json", key.finalize()))
            });
        if let Some(stored) = path.as_ref().and_then(|p| read(p).ok()) {
            return Ok(stored);
        }
        let made = reflect(pass, source)?;
        if let Some(path) = path {
            // A store that cannot be written costs the next process a parse, nothing more.
            let _ = write(&path, &made);
        }
        Ok(made)
    })
}

/// One parse: the selected entry's inputs and, for compute, its workgroup size.
fn reflect(pass: Pass, source: &str) -> Result<Reflected> {
    let mut module = super::parse(source)?;
    super::select(&mut module, pass)?;
    let inputs = super::heap_interface::from_module(&module).map_err(|e| format!("{e:#}"));
    let threads = if pass.raster() {
        None
    } else {
        let entry = module
            .entry_points
            .iter()
            .find(|entry| entry.name == pass.entry())
            .context("selected WGSL entry missing")?;
        ensure!(
            entry.stage == naga::ShaderStage::Compute
                && entry.workgroup_size.iter().all(|v| *v > 0),
            "selected pass is not a concrete compute entry"
        );
        Some(entry.workgroup_size)
    };
    Ok(Reflected { inputs, threads })
}

/// The storage formats reflection can give (`heap_interface::storage_format`).
const FORMATS: [Format; 6] = [
    Format::R32Float,
    Format::Rgba16Float,
    Format::Rgba32Float,
    Format::R32Uint,
    Format::Rg32Uint,
    Format::Rgba32Uint,
];
const DIMENSIONS: [ViewDimension; 4] = [
    ViewDimension::D2,
    ViewDimension::D2Array,
    ViewDimension::D3,
    ViewDimension::CubeArray,
];
const SAMPLES: [SampleType; 3] = [SampleType::Float, SampleType::Uint, SampleType::Depth];
/// The member of `all` whose debug name is `name`.
fn named<T: Copy + std::fmt::Debug>(all: &[T], name: &Value) -> Result<T> {
    let name = name.as_str().context("reflection record: a name")?;
    all.iter()
        .copied()
        .find(|v| format!("{v:?}") == name)
        .with_context(|| format!("reflection record: unknown {name}"))
}

fn write(path: &std::path::Path, reflected: &Reflected) -> Result<()> {
    let Ok(listed) = &reflected.inputs else {
        let record = json!({"inputs_error": reflected.inputs.as_ref().err(),
            "threads": reflected.threads});
        return store(path, &record);
    };
    let inputs: Vec<Value> = listed
        .iter()
        .map(|input| {
            let kind = match input.kind {
                InputKind::AccelerationStructure => json!({"kind": "AccelerationStructure"}),
                InputKind::Buffer { minimum, writable } => {
                    json!({"kind": "Buffer", "minimum": minimum, "writable": writable})
                }
                InputKind::BufferArray {
                    count,
                    minimum,
                    writable,
                } => json!({"kind": "BufferArray", "count": count, "minimum": minimum, "writable": writable}),
                InputKind::Image { dimension, sample } => json!({"kind": "Image",
                    "dimension": format!("{dimension:?}"), "sample": format!("{sample:?}")}),
                InputKind::ImageArray {
                    count,
                    dimension,
                    sample,
                } => json!({"kind": "ImageArray", "count": count,
                    "dimension": format!("{dimension:?}"), "sample": format!("{sample:?}")}),
                InputKind::SamplerArray { count, comparison } => {
                    json!({"kind": "SamplerArray", "count": count, "comparison": comparison})
                }
                InputKind::StorageImage { dimension, format } => json!({"kind": "StorageImage",
                    "dimension": format!("{dimension:?}"), "format": format!("{format:?}")}),
                InputKind::Sampler { comparison } => {
                    json!({"kind": "Sampler", "comparison": comparison})
                }
            };
            json!({"group": input.key.group, "binding": input.key.binding,
                "name": input.name, "kind": kind})
        })
        .collect();
    store(
        path,
        &json!({"inputs": inputs, "threads": reflected.threads}),
    )
}
fn store(path: &std::path::Path, record: &Value) -> Result<()> {
    let dir = path.parent().context("reflection store directory")?;
    std::fs::create_dir_all(dir)?;
    // Write a whole file and rename it over the record, so a reader sees an old or a
    // new record, never a part of one.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let scratch = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&scratch, serde_json::to_vec(record)?)?;
    std::fs::rename(&scratch, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&scratch);
    })?;
    Ok(())
}

fn read(path: &std::path::Path) -> Result<Reflected> {
    let record: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let number = |v: &Value, field: &str| -> Result<u32> {
        let n = v[field]
            .as_u64()
            .with_context(|| format!("reflection record: {field}"))?;
        Ok(u32::try_from(n)?)
    };
    let flag = |v: &Value, field: &str| -> Result<bool> {
        v[field]
            .as_bool()
            .with_context(|| format!("reflection record: {field}"))
    };
    let inputs = if let Some(error) = record["inputs_error"].as_str() {
        Err(error.to_owned())
    } else {
        Ok(record["inputs"]
            .as_array()
            .context("reflection record: inputs")?
            .iter()
            .map(|input| {
                let k = &input["kind"];
                let kind = match k["kind"].as_str().context("reflection record: kind")? {
                    "AccelerationStructure" => InputKind::AccelerationStructure,
                    "Buffer" => InputKind::Buffer {
                        minimum: number(k, "minimum")?,
                        writable: flag(k, "writable")?,
                    },
                    "BufferArray" => InputKind::BufferArray {
                        count: number(k, "count")?,
                        minimum: number(k, "minimum")?,
                        writable: flag(k, "writable")?,
                    },
                    "Image" => InputKind::Image {
                        dimension: named(&DIMENSIONS, &k["dimension"])?,
                        sample: named(&SAMPLES, &k["sample"])?,
                    },
                    "ImageArray" => InputKind::ImageArray {
                        count: number(k, "count")?,
                        dimension: named(&DIMENSIONS, &k["dimension"])?,
                        sample: named(&SAMPLES, &k["sample"])?,
                    },
                    "SamplerArray" => InputKind::SamplerArray {
                        count: number(k, "count")?,
                        comparison: flag(k, "comparison")?,
                    },
                    "StorageImage" => InputKind::StorageImage {
                        dimension: named(&DIMENSIONS, &k["dimension"])?,
                        format: named(&FORMATS, &k["format"])?,
                    },
                    "Sampler" => InputKind::Sampler {
                        comparison: flag(k, "comparison")?,
                    },
                    other => anyhow::bail!("reflection record: unknown kind {other}"),
                };
                Ok(ShaderInput {
                    key: InputKey {
                        group: number(input, "group")?,
                        binding: number(input, "binding")?,
                    },
                    name: input["name"]
                        .as_str()
                        .context("reflection record: name")?
                        .to_owned(),
                    kind,
                })
            })
            .collect::<Result<Vec<_>>>()?)
    };
    let threads = match &record["threads"] {
        Value::Null => None,
        v => {
            let t = v.as_array().context("reflection record: threads")?;
            ensure!(t.len() == 3, "reflection record: threads");
            let t: Vec<u32> = t
                .iter()
                .map(|n| {
                    Ok(u32::try_from(
                        n.as_u64().context("reflection record: threads")?,
                    )?)
                })
                .collect::<Result<_>>()?;
            Some([t[0], t[1], t[2]])
        }
    };
    Ok(Reflected { inputs, threads })
}

#[cfg(test)]
mod tests {
    /// Every built-in compute pass's reflection reads back as it was written.
    #[test]
    fn a_stored_reflection_reads_back_as_written() {
        let dir = std::env::temp_dir().join(format!("renderlab-reflection-{}", std::process::id()));
        for pass in [
            super::Pass::TemporalResolve,
            super::Pass::ProgramBins,
            super::Pass::GiCache(crate::backend_ara::gi_cache::Entry::Setup),
            super::Pass::PrimaryInteractive(
                crate::backend_ara::primary_trace::Interactive::Deferred,
            ),
        ] {
            let made = super::reflect(pass, pass.source().as_ref()).expect("reflects");
            let inputs = made.inputs.clone().expect("packet inputs");
            let path = dir.join(format!("{pass:?}.json"));
            super::write(&path, &made).expect("writes");
            let back = super::read(&path).expect("reads");
            assert_eq!(back.threads, made.threads, "{pass:?}");
            assert!(!inputs.is_empty(), "{pass:?} reflected no inputs");
            let back = back.inputs.expect("packet inputs read back");
            assert_eq!(back.len(), inputs.len(), "{pass:?}");
            for (a, b) in back.iter().zip(&inputs) {
                assert!(
                    a.key == b.key && a.name == b.name && a.kind == b.kind,
                    "{pass:?}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
