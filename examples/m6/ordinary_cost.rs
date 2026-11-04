//! Headless, bounded ordinary submissions. Controls deliberately alter the image.
use super::{cameras::Cameras, prepared::Prepared};
use renderlab::*;
use serde_json::{Value, json};
use std::{
    error::Error,
    fs,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
enum Control {
    Candidate,
    SampleNoQuery,
    SampleNoQueryNoEnvironment,
    PixelNoQuery,
}
impl Control {
    fn source(self, original: &str) -> Result<String, Box<dyn Error>> {
        let call = "attenuation*=hard_shadow(light,input.world,geometric,l);";
        let environment = "if lighting.map.w!=0.0 {";
        if original.matches(call).count() != 1 || original.matches(environment).count() != 1 {
            return Err("diagnostic shader seams changed".into());
        }
        let source = match self {
            Self::Candidate => original.to_owned(),
            _ => original.replace(call, "attenuation*=1.0;"),
        };
        Ok(if matches!(self, Self::SampleNoQueryNoEnvironment) {
            source.replace(environment, "if false {")
        } else {
            source
        })
    }
}

fn ms(value: Duration) -> f64 {
    value.as_secs_f64() * 1000.0
}
fn cpu(value: CpuFrame) -> Value {
    json!({"preparation":ms(value.preparation),"visibility":ms(value.visibility),
        "packing":ms(value.packing),"upload":ms(value.upload),"encode":ms(value.encode),
        "submit":ms(value.submit),"poll":ms(value.poll),
        "shadow_preparation":ms(value.shadow_preparation),"shadow_upload":ms(value.shadow_upload),
        "shadow_encode":ms(value.shadow_encode)})
}
fn uploads(u: UploadBytes) -> Value {
    json!({"geometry":u.geometry,"textures":u.textures,"environment":u.environment,
        "shadows":u.shadows,"camera":u.camera,"lights":u.lights,"lighting":u.lighting,
        "models":u.models,"instances":u.instances,"normals":u.normals,"materials":u.materials,
        "visible_ids":u.visible_ids,"view_settings":u.view_settings,"write_calls":u.write_calls})
}
fn snapshot(r: &Renderer) -> Value {
    let s = r.stats();
    json!({"stats":format!("{s:?}"),"uploads":uploads(s.uploads),
        "persistent_created":s.ownership.created(),"attachment_views_created":s.ownership.attachment_views.created,
        "target_creations":s.ownership.target_creations,"shadow_creations":s.ownership.shadow_creations,
        "pipeline_installations":s.pipelines.installations,"submissions":s.submissions,
        "busy_frames":s.busy_frames,"gpu_samples":s.gpu_samples,"timing_failures":s.timing_failures,
        "events_dropped":s.events_dropped,"peak_flights":s.peak_flights,
        "hardware":r.hardware_shadow_stats().map(|h|json!({"blas_builds":h.blas_builds,
            "tlas_builds":h.tlas_builds,"pending_builds":h.pending_builds,
            "observed_as_bytes":h.observed_as_bytes})),
        "shadow_stats":format!("{:?}",r.shadow_stats())})
}

#[derive(Default)]
struct Trace {
    gpu: Vec<Value>,
    completions: Vec<Value>,
    poll_ms: f64,
    yield_ms: f64,
}
impl Trace {
    fn poll(&mut self, r: &mut Renderer) -> Result<(), Box<dyn Error>> {
        let start = Instant::now();
        let events = r.poll()?;
        self.poll_ms += ms(start.elapsed());
        if events.dropped != 0 {
            return Err("timing events were dropped".into());
        }
        for g in events.gpu {
            self.gpu
                .push(json!({"frame":g.frame.0,"mesh_ms":ms(g.mesh_pass),
                "span_ms":ms(g.render_span),"present_ms":g.present_pass.map(ms),
                "shadow_generation_ms":g.shadow_generation.map(ms)}));
        }
        for c in events.completed {
            self.completions.push(json!({"frame":c.frame.map(|f|f.0),
                "submit_to_observed_ms":ms(c.observed_at.duration_since(c.submitted_at))}));
        }
        Ok(())
    }
    fn yield_now(&mut self) {
        let start = Instant::now();
        std::thread::yield_now();
        self.yield_ms += ms(start.elapsed());
    }
}

fn block(
    r: &mut Renderer,
    color: &wgpu::Texture,
    asset: &mut Prepared,
    cameras: &Cameras,
    frames: usize,
    moving: bool,
    deadline: Instant,
) -> Result<Value, Box<dyn Error>> {
    let before = snapshot(r);
    let mut trace = Trace::default();
    let mut rows = Vec::with_capacity(frames);
    let begin = Instant::now();
    for frame in 0..frames {
        let preparation = Instant::now();
        let pose = if moving {
            frame % cameras.frames.len()
        } else {
            0
        };
        let camera = cameras.frames[pose].camera()?;
        let mut draws = asset.draws(Visibility::Frustum);
        let caller_preparation = ms(preparation.elapsed());
        let admission = Instant::now();
        let mut rejected_ms = 0.0;
        let mut retries = 0usize;
        let (report, accepted_ms) = loop {
            let attempt = Instant::now();
            let result = r.try_render_view(color, &camera, &mut draws, ViewMode::Lit)?;
            let elapsed = ms(attempt.elapsed());
            if let Some(report) = result {
                break (report, elapsed);
            }
            rejected_ms += elapsed;
            retries += 1;
            trace.poll(r)?;
            if Instant::now() > deadline {
                return Err("ordinary admission deadline".into());
            }
            trace.yield_now();
        };
        let admission_ms = ms(admission.elapsed());
        if !report.timed || report.scene.triangles != 13_034_903 || report.scene.draws != 12 {
            return Err(format!("ordinary workload changed: {:?}", report.scene).into());
        }
        rows.push(json!({"frame":report.frame.0,"pose":pose,"caller_preparation_ms":caller_preparation,
            "accepted_render_ms":accepted_ms,"admission_ms":admission_ms,"rejected_attempt_ms":rejected_ms,
            "retries":retries,"cpu_ms":cpu(report.cpu),"uploads":uploads(report.uploads),
            "triangles":report.scene.triangles,"draws":report.scene.draws}));
        trace.poll(r)?;
    }
    let submissions_ms = ms(begin.elapsed());
    let drain = Instant::now();
    while trace.gpu.len() < frames {
        trace.poll(r)?;
        if Instant::now() > deadline {
            return Err("ordinary completion deadline".into());
        }
        if trace.gpu.len() < frames {
            trace.yield_now();
        }
    }
    let drain_ms = ms(drain.elapsed());
    let elapsed_ms = ms(begin.elapsed());
    if trace.gpu.len() != frames || trace.completions.len() != frames {
        return Err("ordinary frame/timing count mismatch".into());
    }
    Ok(
        json!({"frames":frames,"moving":moving,"elapsed_ms":elapsed_ms,
        "drained_ms_per_frame":elapsed_ms/frames as f64,"submission_loop_ms":submissions_ms,
        "final_drain_ms":drain_ms,"explicit_poll_ms":trace.poll_ms,"yield_ms":trace.yield_ms,
        "before":before,"after":snapshot(r),"rows":rows,"gpu":trace.gpu,"completions":trace.completions}),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    r: &mut Renderer,
    color: &wgpu::Texture,
    asset: &mut Prepared,
    cameras: &Cameras,
    lighting: &Lighting,
    shadows: &Shadows,
    original: &str,
    frames: usize,
    out: &Path,
) -> Result<(), Box<dyn Error>> {
    if cameras.frames.len() != 2
        || cameras
            .frames
            .iter()
            .any(|p| p.width != 1920 || p.height != 1080)
    {
        return Err("ordinary screen requires two registered 1920x1080 poses".into());
    }
    let deadline = Instant::now() + Duration::from_secs(150);
    while r.stats().flights > 0 {
        r.poll()?;
        if Instant::now() > deadline {
            return Err("startup upload deadline".into());
        }
        std::thread::yield_now();
    }
    r.poll()?;
    let startup = block(r, color, asset, cameras, 1, false, deadline)?;
    fs::write(
        out.join("ordinary-startup.json"),
        serde_json::to_vec_pretty(&startup)?,
    )?;
    let controls = [
        Control::Candidate,
        Control::SampleNoQuery,
        Control::SampleNoQueryNoEnvironment,
        Control::PixelNoQuery,
    ];
    for repetition in 0..2 {
        for index in 0..controls.len() {
            let control = controls[if repetition == 0 {
                index
            } else {
                controls.len() - 1 - index
            }];
            let source = control.source(original)?;
            let prefix = format!("block-{repetition}-{control:?}");
            fs::write(out.join(format!("{prefix}.wgsl")), &source)?;
            let reload = r.reload(&source)?;
            r.set_lighting_and_shadows(
                lighting,
                &if matches!(control, Control::PixelNoQuery) {
                    Shadows::default()
                } else {
                    shadows.clone()
                },
            )?;
            let warmup = block(r, color, asset, cameras, 6, false, deadline)?;
            fs::write(
                out.join(format!("{prefix}-warmup.json")),
                serde_json::to_vec_pretty(&warmup)?,
            )?;
            for moving in [false, true] {
                let result = block(r, color, asset, cameras, frames, moving, deadline)?;
                let file = out.join(format!(
                    "{prefix}-{}.json",
                    if moving { "motion" } else { "static" }
                ));
                fs::write(
                    &file,
                    serde_json::to_vec_pretty(&json!({"control":format!("{control:?}"),
                    "repetition":repetition,"reload_ms":ms(reload.compile),"result":result}))?,
                )?;
                eprintln!(
                    "ordinary block={prefix} moving={moving} output={}",
                    file.display()
                );
            }
        }
    }
    r.reload(original)?;
    r.set_lighting_and_shadows(lighting, shadows)?;
    Ok(())
}
