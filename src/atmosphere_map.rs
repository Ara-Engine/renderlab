//! RT fog's own single sun map. Surface shadows keep their original ray path.
use super::super::shadow_gpu::{ShadowDraw, ShadowFrame, ShadowState};
use super::*;

pub(super) struct FogMap {
    pub(super) state: ShadowState,
    pub(super) frame: Option<ShadowFrame>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn rt_map_uses_raster_bindings_and_rt_options_preserve_nonrt() {
        use super::super::{kernel_source_for, options::Mode};
        let source = kernel_source_for(true, Mode::Map);
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let bindings: Vec<_> = module
            .global_variables
            .iter()
            .filter_map(|(_, g)| g.binding.as_ref().map(|b| b.binding))
            .collect();
        assert!(bindings.contains(&21) && bindings.contains(&22));
        assert!(!bindings.contains(&20));
        let nonrt = kernel_source_for(false, Mode::Default);
        assert_eq!(kernel_source_for(false, Mode::Map), nonrt);
        assert_eq!(kernel_source_for(false, Mode::Quarter), nonrt);
    }
}

impl Renderer {
    pub(in crate::renderer) fn atmosphere_map_requested(&self) -> bool {
        self.hardware_shadows.is_some()
            && options::mode() == options::Mode::Map
            && self.atmosphere.settings.volumetric_fog.is_some()
    }

    pub(super) fn prepare_fog_map(
        &mut self,
        camera: &Camera,
        size: [u32; 2],
        casters: &[ShadowDraw<'_>],
        uploads: &mut UploadBytes,
    ) -> Result<(), Error> {
        if !self.atmosphere_map_requested() {
            return Ok(());
        }
        let distance = self.atmosphere.settings.volumetric_fog.unwrap().distance;
        let sun = self
            .lighting
            .source_lights
            .iter()
            .find(|l| matches!(l, Light::Directional { .. }))
            .cloned();
        if self.view.atmosphere.shadow_map.is_none() {
            let mut state = self.view.shadows.fork(&self.device, &self.ledger, 0);
            state.settings.resolution = 1024.min(state.limits.max_resolution);
            self.view.atmosphere.shadow_map = Some(FogMap { state, frame: None });
        }
        let map = self.view.atmosphere.shadow_map.as_mut().unwrap();
        map.state.collect(self.completed_submission);
        let Some(sun) = sun else {
            map.frame = None;
            return Ok(());
        };
        let mut bounds = Vec::with_capacity(casters.len());
        let mut casts = Vec::with_capacity(casters.len());
        for chunk in casters
            .chunk_by(|a, b| std::ptr::eq(a.mesh, b.mesh) && std::ptr::eq(a.instances, b.instances))
        {
            let first = &chunk[0];
            let b = first.instances.state.world_bounds(first.mesh.bounds)?;
            for caster in chunk {
                bounds.push(b[caster.slot as usize]);
                casts.push(caster.instances.state.casts_shadows(caster.slot as usize));
            }
        }
        // The fog starts at the eye, independently of the surface camera's near plane.
        let world = camera.view().inverse();
        let eye = world.w_axis.truncate();
        let fog_camera = Camera::look_at(
            eye,
            eye - world.z_axis.truncate(),
            world.y_axis.truncate(),
            camera.fov_y(),
            (distance * 1e-6).max(f32::MIN_POSITIVE),
            distance,
        )?;
        map.state.settings.requests = vec![ShadowRequest::Directional {
            split_ends: vec![distance],
        }];
        map.state
            .settings
            .validate(std::slice::from_ref(&sun), map.state.limits)?;
        let frame = map.state.plan(&fog_camera, size, &[sun], &bounds, &casts)?;
        if let Err(message) = map.state.upload_frame(
            &frame,
            (&self.device, &self.queue, &self.ledger),
            true,
            self.submissions,
            uploads,
        ) {
            return Err(self.fail(message));
        }
        // Sampling covers the eye as well; rasterization retains the planner's finite matrix.
        self.queue
            .write_buffer(map.state.records(), 64, &0.0f32.to_ne_bytes());
        uploads.shadows += 4;
        uploads.write_calls += 1;
        map.state.remember_bases(&frame);
        map.frame = Some(frame);
        Ok(())
    }

    pub(super) fn encode_fog_map(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        casters: &[ShadowDraw<'_>],
        mark: &mut dyn FnMut(&mut wgpu::CommandEncoder, &'static str),
    ) {
        if !self.atmosphere_map_requested() {
            return;
        }
        let Some(map) = self.view.atmosphere.shadow_map.as_ref() else {
            return;
        };
        let Some(frame) = &map.frame else {
            return;
        };
        mark(encoder, "atmosphere.shadow_map");
        self.encode_shadow_state(encoder, &map.state, frame, casters, None);
        // Close map upkeep before the sky LUTs; their cost belongs to atmosphere.
        mark(encoder, "atmosphere");
    }
}
