//! Measurement: `RENDERLAB_LIVE_REFERENCE=<dir>` path-traces the engine's own scene at an armed
//! frame (docs/design-reference-live.md), so every material program is traced as the engine runs
//! it. Armed like the scene dump: once `<dir>/arm` exists (a label line), the next view rendered
//! is traced into `<dir>/<label>/`; `RENDERLAB_LIVE_REFERENCE=<dir>;<frame>,...` also traces the
//! listed renderer frames as `frame-<id>`, for frame-locked hosts such as a replay.
//! `RENDERLAB_LIVE_REFERENCE_SAMPLES` sets the samples per pixel (default 16384), and
//! `RENDERLAB_LIVE_REFERENCE_DEPTH` the path segments (default 1024; 2 is the direct light at the
//! visible surface, src/reference_live.rs `max_depth`), to split a source's light by bounce.
//! Each traced view writes `reference.pfm` (the linear mean, cd/m^2, rows bottom to top),
//! `reference-half0.pfm` and `-half1.pfm` (independent halves), `reference.png` (the mean through
//! AcesFit and sRGB at the frame's exposure) and `reference.json` (main-thread/ref/README.md's
//! fields: samples, seconds, invalid pixels, size, exposure, method, `not_traced`, `sky_check`,
//! `masked_fraction`). It needs ray queries and a traced frame before it (hardware shadows on).
use super::*;
use std::{collections::HashMap, path::PathBuf};

#[derive(Default)]
pub(super) struct LiveReference {
    dir: PathBuf,
    frames: Vec<u64>,
    samples: u32,
    depth: u32,
    environments: HashMap<EnvironmentId, Arc<PreparedEnvironment>>,
}

impl LiveReference {
    pub(super) fn from_env() -> Option<Self> {
        let value = std::env::var("RENDERLAB_LIVE_REFERENCE").ok()?;
        let (dir, frames) = match value.split_once(';') {
            Some((dir, list)) => (dir, list.split(',').filter_map(|f| f.trim().parse().ok()).collect()),
            None => (value.as_str(), vec![]),
        };
        let samples = std::env::var("RENDERLAB_LIVE_REFERENCE_SAMPLES").ok().and_then(|s| s.parse().ok()).unwrap_or(16384);
        let depth = std::env::var("RENDERLAB_LIVE_REFERENCE_DEPTH").ok().and_then(|s| s.parse().ok()).unwrap_or(1024);
        Some(Self { dir: dir.into(), frames, samples, depth, environments: HashMap::new() })
    }
    pub(super) fn environment(&mut self, id: EnvironmentId, data: &PreparedEnvironment) {
        self.environments.insert(id, Arc::new(data.clone()));
    }
    /// The label renderer frame `frame` traces under: `frame-<id>` when listed, else the label
    /// in `<dir>/arm` (consumed) once its line is complete.
    pub(super) fn armed(&self, frame: u64) -> Option<String> {
        if self.frames.contains(&frame) {
            return Some(format!("frame-{frame}"));
        }
        let arm = self.dir.join("arm");
        let text = std::fs::read_to_string(&arm).ok().filter(|t| t.ends_with('\n'))?;
        let _ = std::fs::remove_file(&arm);
        Some(text.lines().next().unwrap_or("frame").trim().to_owned())
    }
}

/// A binary PFM of linear RGB, rows bottom to top (also the armed frame readback's HDR image).
pub(super) fn pfm(path: &std::path::Path, size: [u32; 2], pixels: &[[f32; 3]]) -> Result<(), Error> {
    let mut bytes = format!("PF\n{} {}\n-1\n", size[0], size[1]).into_bytes();
    for row in (0..size[1] as usize).rev() {
        for p in &pixels[row * size[0] as usize..(row + 1) * size[0] as usize] {
            for c in p {
                bytes.extend(c.to_le_bytes());
            }
        }
    }
    std::fs::write(path, bytes).map_err(|_| Error::InvalidInput("live reference write failed"))
}
/// The mean through the views' comparison display (main-thread/ref/README.md: AcesFit per channel,
/// sRGB) at `stops`, as a binary PPM (no image dependency in the library).
fn ppm(path: &std::path::Path, size: [u32; 2], pixels: &[[f32; 3]], stops: f32) -> Result<(), Error> {
    let mut bytes = format!("P6\n{} {}\n255\n", size[0], size[1]).into_bytes();
    let scale = 2f32.powf(stops);
    for p in pixels {
        for &c in p {
            let x = (c * scale).max(0.0);
            let m = ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0);
            let s = if m <= 0.003_130_8 { 12.92 * m } else { 1.055 * m.powf(1.0 / 2.4) - 0.055 };
            bytes.push((s * 255.0).round() as u8);
        }
    }
    std::fs::write(path, bytes).map_err(|_| Error::InvalidInput("live reference write failed"))
}

impl Engine {
    /// Traces the scene of the frame just rendered for `label` as seen by `camera` (module doc).
    pub(super) fn trace_live(&mut self, label: &str, camera: &Camera, output: &wgpu::Texture, stops: f32) -> Result<(), Error> {
        if self.live.is_none() {
            return Ok(());
        }
        let size = [output.width(), output.height()];
        // One more frame of the same view with the transparent draws kept traced, so the TLAS, the
        // shading groups and the per-program materials include see-through surfaces.
        self.renderer.set_live_opaque_frame(true);
        let frame = self.render_selected(output, camera, ViewMode::Lit);
        self.renderer.set_live_opaque_frame(false);
        frame?;
        let Some(live) = self.live.as_ref() else { return Ok(()) };
        let out = live.dir.join(label);
        let _ = std::fs::create_dir_all(&out);
        let inverse = camera.view().inverse();
        let eye = inverse.w_axis.truncate();
        let forward = -inverse.z_axis.truncate().normalize();
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward);
        let mut not_traced: Vec<String> = Vec::new();
        // Under the atmosphere the sky is the procedural cube shading reads, and the sun's
        // illuminance is what reaches surfaces (scene_dump.rs).
        let procedural = matches!(self.state.sky, Sky::Atmosphere(_)).then(|| self.renderer.procedural_sky_dump(eye)).flatten();
        let transmittance = procedural.as_ref().map_or([1.0; 3], |p| p.2);
        let sun = self.state.sun.map(|s| crate::reference::Sun {
            to_light: -s.travel.normalize(),
            illuminance: std::array::from_fn(|c| s.illuminance[c] * transmittance[c]),
            angular_radius: s.angular_radius,
        });
        let (sky, environment) = match self.state.sky {
            Sky::Uniform(r) => (r, None),
            Sky::Map { map, rotation_y, intensity, .. } => match live.environments.get(&map) {
                Some(e) => (
                    [0.0; 3],
                    Some(crate::reference::Environment {
                        radiance: e.specular_radiance()[0].clone(),
                        yaw: e.metadata().rotation_y_radians + rotation_y,
                        intensity,
                    }),
                ),
                None => return Err(Error::InvalidInput("live reference: the sky map was created before RENDERLAB_LIVE_REFERENCE was read")),
            },
            Sky::Atmosphere(_) => match procedural {
                Some((n, texels, _)) => {
                    not_traced.push("atmosphere: sky from the engine's 128-texel cube".into());
                    let level = crate::asset::environment::CubeLevel::new(n, texels, crate::asset::environment::Limits::unbounded())
                        .map_err(|_| Error::InvalidInput("live reference: the procedural sky readback is malformed"))?;
                    ([0.0; 3], Some(crate::reference::Environment { radiance: level, yaw: 0.0, intensity: 1.0 }))
                }
                None => return Err(Error::InvalidInput("live reference: the procedural sky has not been written yet")),
            },
        };
        let (mut lights, mut light_radii) = (Vec::new(), Vec::new());
        for l in &self.state.lights {
            match *l {
                Light::Point { position, candela, range, source_radius, .. } => {
                    lights.push(crate::reference::LocalLight { position, candela, range: range.unwrap_or(0.0), spot: None });
                    light_radii.push(source_radius);
                }
                Light::Spot { position, travel, candela, range, inner, outer, source_radius, .. } => {
                    lights.push(crate::reference::LocalLight { position, candela, range: range.unwrap_or(0.0), spot: Some((travel, inner.cos(), outer.cos())) });
                    light_radii.push(source_radius);
                }
                Light::Directional { .. } => not_traced.push("a directional light other than the sun".into()),
            }
        }
        not_traced.push("see-through surfaces as thin films and thin-slab glass: no refraction offset (thickness ignored)".into());
        let atmosphere = self.renderer.atmosphere();
        if atmosphere.aerial_perspective {
            not_traced.push("aerial perspective: the air between surfaces (about 1% blue extinction at 300 m)".into());
        }
        if atmosphere.height_fog.is_some_and(|f| f.start_distance > 0.0) {
            not_traced.push("the height fog's start distance (fog is traced from the eye)".into());
        }
        // Measurement (RENDERLAB_ABLATE `sun`, `sky`, `lights`, as Engine::apply_lighting takes them
        // out of the frame): the reference drops the same sources, so one light term is judged on
        // both sides. The embedder's state, which this reads, keeps them. (gicut, be6ae308)
        let ablate = |feature: &str| {
            std::env::var("RENDERLAB_ABLATE").is_ok_and(|list| list.split(',').any(|f| f.trim() == feature))
        };
        let sun = if ablate("sun") { None } else { sun };
        if ablate("lights") {
            lights.clear();
            light_radii.clear();
        }
        let (sky, environment) = if ablate("sky") {
            ([0.0; 3], environment.map(|e| crate::reference::Environment { intensity: 0.0, ..e }))
        } else {
            (sky, environment)
        };
        let inputs = crate::reference_live::LiveInputs {
            eye,
            forward,
            right,
            up,
            tan_half_fov: (camera.fov_y() * 0.5).tan(),
            size,
            sun,
            sky,
            environment,
            lights,
            light_radii,
            samples: live.samples,
            max_depth: live.depth,
            seed: 0x4d37_5530,
            progress: true,
            fog: crate::reference_live::Fog::of(&atmosphere),
        };
        let samples = inputs.samples;
        let depth = inputs.max_depth;
        // The traced medium, recorded with the picture (null: none).
        let fog = inputs.fog.map_or("null".to_string(), |f| format!("{{\"density\":{},\"falloff\":{},\"base_height\":{},\"extra_density\":{},\"extra_radius\":{},\"albedo\":[{},{},{}],\"anisotropy\":{}}}", f.density, f.falloff, f.base_height, f.extra_density, f.extra_radius, f.albedo[0], f.albedo[1], f.albedo[2], f.anisotropy));
        let started = std::time::Instant::now();
        let image = self.renderer.trace_live_reference(&inputs)?;
        let seconds = started.elapsed().as_secs_f64();
        pfm(&out.join("reference.pfm"), size, &image.mean)?;
        for (k, half) in image.halves.iter().enumerate() {
            pfm(&out.join(format!("reference-half{k}.pfm")), size, half)?;
        }
        ppm(&out.join("reference.ppm"), size, &image.mean, stops)?;
        let list = not_traced.iter().map(|g| format!("{g:?}")).collect::<Vec<_>>().join(",");
        let json = format!(
            "{{\"samples\":{samples},\"max_depth\":{depth},\"seconds\":{seconds},\"invalid_pixels\":{},\"size\":[{},{}],\"exposure_stops\":{stops},\"display_transform\":\"AcesFit+sRGB at the engine view's exposure\",\"method\":\"renderlab::reference_live: the engine's own scene and material programs, unidirectional path tracing, NEE + MIS, Russian roulette; fog as a physical medium on every ray; no reuse or denoising\",\"fog\":{fog},\"not_traced\":[{list}],\"sky_check\":null,\"masked_fraction\":0,\"camera\":{{\"eye\":[{},{},{}],\"forward\":[{},{},{}],\"fov_y\":{}}},\"path\":\"{:?}\"}}\n",
            image.invalid_pixels, size[0], size[1], eye.x, eye.y, eye.z, forward.x, forward.y, forward.z, camera.fov_y(), self.path
        );
        std::fs::write(out.join("reference.json"), json).map_err(|_| Error::InvalidInput("live reference write failed"))?;
        eprintln!("live_reference {label}: {samples} samples in {seconds:.1} s");
        Ok(())
    }
}
