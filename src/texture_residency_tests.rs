//! M3 texture mip residency: the hardware-sampling facts the design rests on, and the
//! production residency checks. Nested under visibility_buffer_tests for its fixtures.
use super::*;

/// Probe edge length: level j of the shared probe holds 2^-j (`PROBE_LEVELS` levels).
const PROBE: u32 = 64;

/// xorshift64* bytes: independent random content per level makes every filter weight
/// visible in the sampled result.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut s = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            (s.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 56) as u8
        })
        .collect()
}

fn probe_device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::empty().with_env(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("M3 probe adapter {:?}", adapter.get_info());
    let extra = adapter.features() & wgpu::Features::FLOAT32_FILTERABLE;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: extra,
        ..Default::default()
    }))
    .unwrap()
}

/// Uploads `levels[first..]` as a texture whose level 0 is source level `first`.
fn tail_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    levels: &[(u32, u32, Vec<u8>)],
    first: usize,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    let (w, h, _) = &levels[first];
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("M3 probe texture"),
        size: wgpu::Extent3d {
            width: *w,
            height: *h,
            depth_or_array_layers: 1,
        },
        mip_level_count: (levels.len() - first) as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let texel = format.block_copy_size(None).unwrap();
    for (level, (w, h, bytes)) in levels[first..].iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                mip_level: level as u32,
                ..texture.as_image_copy()
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * texel),
                rows_per_image: Some(*h),
            },
            wgpu::Extent3d {
                width: *w,
                height: *h,
                depth_or_array_layers: 1,
            },
        );
    }
    texture
}

/// The full chain of a w x h texture with independent random levels.
fn random_chain(w: u32, h: u32, seed: u64) -> Vec<(u32, u32, Vec<u8>)> {
    let (mut w, mut h) = (w, h);
    let mut levels = vec![];
    loop {
        levels.push((w, h, noise(seed + levels.len() as u64, (w * h * 4) as usize)));
        if w == 1 && h == 1 {
            return levels;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
}

/// A level probe of w x h with its full chain, level j filled with 2^-j; the shared
/// probe is PROBE x PROBE.
fn probe_chain(w: u32, h: u32, format: wgpu::TextureFormat) -> Vec<(u32, u32, Vec<u8>)> {
    let (mut w, mut h) = (w, h);
    let mut levels = vec![];
    loop {
        let j = levels.len() as u32;
        // 2^-j: binary16 exponent field 15 - j with a zero mantissa.
        let texel: Vec<u8> = if format == wgpu::TextureFormat::R16Float {
            (((15 - j) as u16) << 10).to_le_bytes().to_vec()
        } else {
            2f32.powi(-(j as i32)).to_le_bytes().to_vec()
        };
        levels.push((w, h, texel.repeat((w * h) as usize)));
        if w == 1 && h == 1 {
            return levels;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
}

const PROBE_SHADER: &str = r#"
struct Params { probe_scale:vec2<f32>, count:u32, pad:u32 };
@group(0) @binding(0) var<uniform> params:Params;
@group(0) @binding(1) var s:sampler;
@group(0) @binding(2) var full:texture_2d<f32>;
@group(0) @binding(3) var tail1:texture_2d<f32>;
@group(0) @binding(4) var tail2:texture_2d<f32>;
@group(0) @binding(5) var tail3:texture_2d<f32>;
@group(0) @binding(6) var tail4:texture_2d<f32>;
@group(0) @binding(7) var tail5:texture_2d<f32>;
@group(0) @binding(8) var tail6:texture_2d<f32>;
@group(0) @binding(9) var probe16:texture_2d<f32>;
@group(0) @binding(10) var probe32:texture_2d<f32>;
@group(0) @binding(11) var<storage,read> inputs:array<vec4<f32>>;
@group(0) @binding(12) var<storage,read_write> outputs:array<vec4<u32>>;
// A probe with the source's own extent and level count, sampled with the unscaled footprint.
@group(0) @binding(13) var probe_full:texture_2d<f32>;
fn same(a:vec4<f32>,b:vec4<f32>)->bool { return all(bitcast<vec4<u32>>(a)==bitcast<vec4<u32>>(b)); }
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;
    if i>=params.count {return;}
    let a=inputs[2u*i];let b=inputs[2u*i+1u];
    let uv=a.xy;let dx=a.zw;let dy=b.xy;
    let f=textureSampleGrad(full,s,uv,dx,dy);
    var mask=0u;
    if same(textureSampleGrad(tail1,s,uv,dx,dy),f) {mask|=1u;}
    if same(textureSampleGrad(tail2,s,uv,dx,dy),f) {mask|=2u;}
    if same(textureSampleGrad(tail3,s,uv,dx,dy),f) {mask|=4u;}
    if same(textureSampleGrad(tail4,s,uv,dx,dy),f) {mask|=8u;}
    if same(textureSampleGrad(tail5,s,uv,dx,dy),f) {mask|=16u;}
    if same(textureSampleGrad(tail6,s,uv,dx,dy),f) {mask|=32u;}
    let pdx=dx*params.probe_scale;let pdy=dy*params.probe_scale;
    let p16=textureSampleGrad(probe16,s,uv,pdx,pdy).r;
    let p32=textureSampleGrad(probe32,s,uv,pdx,pdy).r;
    let pf=textureSampleGrad(probe_full,s,uv,dx,dy).r;
    outputs[i]=vec4<u32>(bitcast<u32>(p16),bitcast<u32>(p32),mask,bitcast<u32>(pf));
}
"#;

/// Random uv footprints in texel units of level 0: wide random LOD/anisotropy plus
/// isotropic and 4:1 sweeps approaching every integer LOD from both sides.
fn footprints(w: u32, h: u32, count: usize, seed: u64) -> Vec<[f32; 4]> {
    let bytes = noise(seed, count * 16);
    let unit = |i: usize, k: usize| {
        let b = &bytes[i * 16 + k * 2..i * 16 + k * 2 + 2];
        (f32::from(u16::from_le_bytes([b[0], b[1]])) + 0.5) / 65536.0
    };
    let scale = glam::Vec2::new(w as f32, h as f32);
    let mut out = Vec::with_capacity(count * 2);
    for i in 0..count {
        let uv = glam::Vec2::new(unit(i, 0) * 3.0 - 1.0, unit(i, 1) * 3.0 - 1.0);
        let theta = unit(i, 2) * std::f32::consts::TAU;
        let (sin, cos) = theta.sin_cos();
        let (major, minor) = match i % 10 {
            0..=6 => {
                let minor = (unit(i, 3) * 17.0 - 4.0).exp2();
                (minor * (unit(i, 4) * 7.0).exp2(), minor)
            }
            7 | 8 => {
                // Exact boundaries L +- 2^-k, isotropic (7) or ratio 4 (8).
                let level = (unit(i, 3) * 13.0).floor();
                let k = 1 + (unit(i, 4) * 24.0) as i32;
                let side = if unit(i, 5) < 0.5 { -1.0 } else { 1.0 };
                let m = (level + side * 2f32.powi(-k)).exp2();
                if i % 10 == 7 { (m, m) } else { (4.0 * m, m) }
            }
            _ => {
                let m = (unit(i, 3) * 15.0 - 2.0).exp2();
                (m * (1.0 + unit(i, 4)), m)
            }
        };
        let skew = unit(i, 6) - 0.5;
        let mut dx = glam::Vec2::new(cos, sin) * major;
        let mut dy = glam::Vec2::new(-sin + skew * cos, cos + skew * sin) * minor;
        if unit(i, 7) < 0.5 {
            std::mem::swap(&mut dx, &mut dy);
        }
        let (dx, dy) = (dx / scale, dy / scale);
        out.push([uv.x, uv.y, dx.x, dx.y]);
        out.push([dy.x, dy.y, 0.0, 0.0]);
    }
    out
}

/// Measure first (M3): for each sampled footprint, does a texture holding only source
/// levels t.. sample bit-identically to the full chain whenever the hardware's finest
/// touched level is at least t, and does the shared probe, sampled with the same
/// sampler and power-of-two-scaled gradients, report that finest level exactly?
#[test]
#[ignore = "M3 owned GPU lease: mip-tail identity and level-probe exactness"]
fn gpu_residency_probe() {
    let out = PathBuf::from(
        std::env::var("RENDERLAB_RESIDENCY_OUT")
            .unwrap_or_else(|_| "E:/renderlab-work/m3-texture/probe".into()),
    );
    fs::create_dir_all(&out).unwrap();
    let (device, queue) = probe_device();
    let filterable32 = device
        .features()
        .contains(wgpu::Features::FLOAT32_FILTERABLE);
    let probe16 = tail_texture(
        &device,
        &queue,
        &probe_chain(PROBE, PROBE, wgpu::TextureFormat::R16Float),
        0,
        wgpu::TextureFormat::R16Float,
    );
    let probe32 = filterable32.then(|| {
        tail_texture(
            &device,
            &queue,
            &probe_chain(PROBE, PROBE, wgpu::TextureFormat::R32Float),
            0,
            wgpu::TextureFormat::R32Float,
        )
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("M3 probe"),
        source: wgpu::ShaderSource::Wgsl(PROBE_SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("M3 probe"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    use wgpu::{FilterMode as F, MipmapFilterMode as M};
    let samplers = [
        ("aniso16-trilinear", F::Linear, F::Linear, M::Linear, 16),
        ("aniso2-trilinear", F::Linear, F::Linear, M::Linear, 2),
        ("trilinear", F::Linear, F::Linear, M::Linear, 1),
    ];
    let count = 1usize << 20;
    let mut report = vec![];
    for (w, h, format) in [
        (4096, 4096, wgpu::TextureFormat::Rgba8UnormSrgb),
        (1024, 1024, wgpu::TextureFormat::Rgba8Unorm),
    ] {
        let chain = random_chain(w, h, u64::from(w * 31 + h));
        let probe_full = tail_texture(
            &device,
            &queue,
            &probe_chain(w, h, wgpu::TextureFormat::R16Float),
            0,
            wgpu::TextureFormat::R16Float,
        );
        let probe_full_view = probe_full.create_view(&Default::default());
        let tails: Vec<_> = (0..7)
            .map(|t| tail_texture(&device, &queue, &chain, t, format))
            .collect();
        let inputs = footprints(w, h, count, u64::from(w + h));
        let input_bytes: Vec<u8> = inputs
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let input = wgpu::util::DeviceExt::create_buffer_init(
            &device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("M3 probe footprints"),
                contents: &input_bytes,
                usage: wgpu::BufferUsages::STORAGE,
            },
        );
        let mut params = [0u8; 16];
        params[0..4].copy_from_slice(&(w as f32 / PROBE as f32).to_le_bytes());
        params[4..8].copy_from_slice(&(h as f32 / PROBE as f32).to_le_bytes());
        params[8..12].copy_from_slice(&(count as u32).to_le_bytes());
        let uniform = wgpu::util::DeviceExt::create_buffer_init(
            &device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("M3 probe params"),
                contents: &params,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let bytes = (count * 16) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("M3 probe results"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("M3 probe readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let views: Vec<_> = tails
            .iter()
            .map(|t| t.create_view(&Default::default()))
            .collect();
        let probe16_view = probe16.create_view(&Default::default());
        let probe32_view = probe32
            .as_ref()
            .map_or_else(|| probe16_view.clone(), |t| t.create_view(&Default::default()));
        for (name, mag, min, mip, anisotropy) in samplers {
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: mag,
                min_filter: min,
                mipmap_filter: mip,
                lod_max_clamp: 32.0,
                anisotropy_clamp: anisotropy,
                ..Default::default()
            });
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ];
            for (i, view) in views.iter().enumerate() {
                entries.push(wgpu::BindGroupEntry {
                    binding: 2 + i as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                });
            }
            entries.extend([
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&probe16_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&probe32_view),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: wgpu::BindingResource::TextureView(&probe_full_view),
                },
            ]);
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("M3 probe"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            let mut e = device.create_command_encoder(&Default::default());
            {
                let mut pass = e.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups((count as u32).div_ceil(64), 1, 1);
            }
            e.copy_buffer_to_buffer(&output, 0, &read, 0, bytes);
            e.map_buffer_on_submit(&read, wgpu::MapMode::Read, .., |result| result.unwrap());
            queue.submit([e.finish()]);
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(Duration::from_secs(60)),
                })
                .unwrap();
            let rows: Vec<[u32; 4]> = read
                .get_mapped_range(..)
                .unwrap()
                .chunks_exact(16)
                .map(|c| std::array::from_fn(|k| u32::from_le_bytes(c[k * 4..k * 4 + 4].try_into().unwrap())))
                .collect();
            read.unmap();
            report.push(analyse(name, w, h, &rows, &inputs, filterable32));
            eprintln!("M3 probe {w}x{h} {name}: {}", report.last().unwrap());
        }
    }
    fs::write(
        out.join("probe.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    // Isotropic filtering: the mip tail and the shared probe are exact (first screen).
    for r in report.iter().filter(|r| r["sampler"] == "trilinear") {
        assert_eq!(r["violations_r16f_d12"], 0, "{r}");
    }
}

/// Decoded finest level at decode offset 2^-`d`: probe values of a blend of levels j and
/// j+1 lie in (2^-(j+1), 2^-j], so floor(-log2 v) is the finer level.
fn decode(value: f32, d: i32) -> u32 {
    let v = f64::from(value);
    ((-v.log2() + 2f64.powi(-d)).floor().clamp(0.0, 15.0)) as u32
}

/// Per probe and decode choice: violations (a level at or above the decode differs when
/// dropped, so dropping it would change the image), and over-reports (the next coarser
/// tail is still identical, so the decode keeps one level more than necessary). The
/// breakdowns locate violations by the footprint's level-0 texel-space anisotropy and by
/// how many tails differ although the full-extent probe says they may be dropped.
fn analyse(
    name: &str,
    w: u32,
    h: u32,
    rows: &[[u32; 4]],
    inputs: &[[f32; 4]],
    filterable32: bool,
) -> serde_json::Value {
    let max_tail = 6u32;
    let mut out = serde_json::json!({ "sampler": name, "width": w, "height": h, "samples": rows.len(), "r32f_probe": filterable32 });
    let mut monotone_breaks = 0u64;
    for row in rows {
        let mask = row[2];
        // Dropping more levels can only stay identical if dropping fewer did.
        let mut seen_break = false;
        for t in 0..max_tail {
            let same = mask & (1 << t) != 0;
            if !same {
                seen_break = true;
            } else if seen_break {
                monotone_breaks += 1;
                break;
            }
        }
    }
    out["mask_monotone_breaks"] = monotone_breaks.into();
    // Level-0 texel-space footprint ratio |major| / |minor| of the derivative vectors.
    let ratio = |i: usize| {
        let a = inputs[2 * i];
        let b = inputs[2 * i + 1];
        let dx = (a[2] * w as f32).hypot(a[3] * h as f32);
        let dy = (b[0] * w as f32).hypot(b[1] * h as f32);
        dx.max(dy) / dx.min(dy).max(1e-30)
    };
    let violated = |level: u32, mask: u32| (1..=level.min(max_tail)).any(|t| mask & (1 << (t - 1)) == 0);
    for (probe, column) in [("r16f", 0usize), ("r32f", 1), ("full", 3)] {
        for d in [30, 12] {
            let (mut violations, mut over, mut histogram) = (0u64, 0u64, [0u64; 16]);
            let mut by_ratio = [0u64; 8];
            let mut by_level = [0u64; 16];
            for (i, row) in rows.iter().enumerate() {
                let level = decode(f32::from_bits(row[column]), d);
                histogram[level as usize] += 1;
                let mask = row[2];
                if violated(level, mask) {
                    violations += 1;
                    by_level[level as usize] += 1;
                    by_ratio[(ratio(i).log2().max(0.0) as usize).min(7)] += 1;
                }
                if level < max_tail && mask & (1 << level) != 0 {
                    over += 1;
                }
            }
            out[format!("violations_{probe}_d{d}")] = violations.into();
            out[format!("over_reports_{probe}_d{d}")] = over.into();
            if d == 12 {
                out[format!("levels_{probe}")] = histogram.to_vec().into();
                out[format!("violations_by_level_{probe}")] = by_level.to_vec().into();
                out[format!("violations_by_log2_ratio_{probe}")] = by_ratio.to_vec().into();
            }
        }
    }
    // Small probe against the full-extent probe: decoded level agreement.
    let disagree = rows
        .iter()
        .filter(|r| decode(f32::from_bits(r[0]), 12) != decode(f32::from_bits(r[3]), 12).min(max_tail))
        .count();
    out["small_vs_full_probe_disagreements"] = disagree.into();
    out
}
