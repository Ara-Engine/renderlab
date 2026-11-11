//! `Sky::Map` on the traced frame: a prepared environment (`PreparedEnvironment`) on the
//! GPU in the layout `Renderer::try_upload_environment` gives it, uploaded by the first
//! frame that names it and kept while frames go on naming it.
//!
//! The irradiance cube binds at (3, 6). An independent-roughness field binds its first
//! 42 knots at (3, 7) and the other 23 at (3, 9), one cube per knot; a legacy mip chain
//! binds its levels at (3, 7) and, where it has levels narrower than `BANK_SIZE`, those
//! levels banked at (3, 9) (P-SHOW2). A field's dominant-lobe table binds at (3, 10).
//! `lighting.map` names the layout, and primary misses read the map when it is visible.
use super::{graph::Graph, image::Texture, transfer};
use crate::asset::environment::{PreparedEnvironment, SpecularLayout};
use anyhow::{Result, ensure};
use ara_gpu::{
    Access, Buffer, Device, GpuDevice, Resource, commands::Recorder, descriptors::Format,
};

/// An independent field's knots in its first array; the rest are in the second.
const FIRST_KNOTS: usize = 42;

/// A frame's sky map and how the frame uses it (`Sky::Map`).
#[derive(Clone, Copy)]
pub struct SkyMap<'a> {
    /// Its identity to the frames, which keep its cubes on the GPU while frames name it.
    /// Never reused for other content.
    pub key: u64,
    pub data: &'a PreparedEnvironment,
    /// Added to the map's own yaw, in radians.
    pub rotation_y: f32,
    /// Its radiance scale where it lights the scene.
    pub intensity: f32,
    /// Primary misses show the map at this radiance scale (`Background::Sky` is the
    /// intensity, `ScaledSky` its own); `None` leaves them the lit clear colour.
    pub visible: Option<f32>,
}

/// A sky map's images on the GPU.
pub(super) struct ResidentMap {
    pub key: u64,
    diffuse: Texture,
    specular: Texture,
    second: Option<Texture>,
    lobes: Option<Texture>,
    /// `lighting.map.w`: -levels for an independent field, levels for a legacy chain,
    /// levels + 0.5 for a banked one.
    layout: f32,
    /// The map's own yaw (`Metadata::rotation_y_radians`).
    yaw: f32,
}

impl ResidentMap {
    /// Makes the map's images and records their uploads; `staging` collects the
    /// buffers to retire once the frame is submitted.
    pub fn upload(
        gpu: &mut GpuDevice,
        graph: &mut Graph,
        rec: &mut Recorder,
        staging: &mut Vec<Buffer>,
        map: &SkyMap<'_>,
    ) -> Result<Self> {
        let data = map.data;
        let pack = crate::environment::pack_level;
        let diffuse_level = data.diffuse_irradiance();
        let diffuse = Texture::cubes(gpu, diffuse_level.size(), 1, Format::Rgba32Float)?;
        staging.push(transfer::upload(
            gpu,
            graph,
            rec,
            diffuse.image,
            &pack(diffuse_level),
        )?);
        let levels = data.specular_radiance();
        let side = levels[0].size();
        let independent = matches!(
            data.specular_layout(),
            SpecularLayout::IndependentRoughness { .. }
        );
        let (specular, second, lobes, layout) = if independent {
            ensure!(
                levels.len() > FIRST_KNOTS,
                "an independent field has more than {FIRST_KNOTS} knots"
            );
            let mut arrays = Vec::new();
            for knots in [&levels[..FIRST_KNOTS], &levels[FIRST_KNOTS..]] {
                let image = Texture::cubes(gpu, side, knots.len() as u32, Format::Rgba32Float)?;
                let bytes: Vec<u8> = knots.iter().flat_map(pack).collect();
                staging.push(transfer::upload(gpu, graph, rec, image.image, &bytes)?);
                arrays.push(image);
            }
            let second = arrays.pop();
            let specular = arrays.pop().expect("two arrays made above");
            let lobes = match data.specular_layout() {
                SpecularLayout::IndependentRoughness { knots, .. } => {
                    // A field without dominant lobes leaves the default table bound.
                    let fitted = crate::environment_lobes::fit(&levels[0]);
                    if fitted.is_empty() {
                        None
                    } else {
                        let texels = crate::environment_lobes::table(&fitted, knots);
                        let width = crate::environment_lobes::SHARE_BINS as u32;
                        let image = Texture::new(
                            gpu,
                            [width, texels.len() as u32 / width],
                            Format::Rgba32Float,
                            false,
                            false,
                        )?;
                        let bytes: Vec<u8> = texels
                            .iter()
                            .flatten()
                            .flat_map(|v| v.to_ne_bytes())
                            .collect();
                        staging.push(transfer::upload(gpu, graph, rec, image.image, &bytes)?);
                        Some(image)
                    }
                }
                SpecularLayout::LegacyMipChain => None,
            };
            (specular, second, lobes, -(levels.len() as f32))
        } else {
            // Its per-level storage views share the chain's allocation; only it retires.
            let (chain, _) = Texture::cube_chain(gpu, side, levels.len() as u32)?;
            for (mip, level) in levels.iter().enumerate() {
                staging.push(transfer::upload_region(
                    gpu,
                    graph,
                    rec,
                    chain.image,
                    mip as u32,
                    [level.size(); 2],
                    6,
                    &pack(level),
                )?);
            }
            let banked = crate::environment::bank_legacy(levels);
            let second = match &banked {
                Some((_, banks)) => {
                    let image = Texture::cubes(
                        gpu,
                        crate::environment::BANK_SIZE,
                        banks.len() as u32,
                        Format::Rgba32Float,
                    )?;
                    staging.push(transfer::upload(
                        gpu,
                        graph,
                        rec,
                        image.image,
                        &banks.concat(),
                    )?);
                    Some(image)
                }
                None => None,
            };
            let layout = levels.len() as f32 + if banked.is_some() { 0.5 } else { 0. };
            (chain, second, None, layout)
        };
        Ok(Self {
            key: map.key,
            diffuse,
            specular,
            second,
            lobes,
            layout,
            yaw: data.metadata().rotation_y_radians,
        })
    }

    /// Declares the images an earlier frame uploaded as read by shading since.
    pub fn import(&self, graph: &mut Graph) {
        for image in self.images() {
            graph.import(Resource::Image(image.image), Access::ShaderRead);
        }
    }

    /// The image this map supplies at a group 3 binding, if any.
    pub fn binding(&self, binding: u32) -> Option<&Texture> {
        match binding {
            6 => Some(&self.diffuse),
            7 => Some(&self.specular),
            9 => self.second.as_ref(),
            10 => self.lobes.as_ref(),
            _ => None,
        }
    }

    /// `lighting.map` for this map under `map`'s rotation and intensity, as
    /// `EnvironmentState::uniform` packs it.
    pub fn words(&self, map: &SkyMap<'_>) -> [f32; 4] {
        // Summed in f64 so two finite angles cannot overflow before the reduction.
        let angle =
            (f64::from(self.yaw) + f64::from(map.rotation_y)).rem_euclid(std::f64::consts::TAU);
        let (sin, cos) = angle.sin_cos();
        [cos as f32, sin as f32, map.intensity, self.layout]
    }

    fn images(&self) -> impl Iterator<Item = &Texture> {
        [Some(&self.diffuse), Some(&self.specular)]
            .into_iter()
            .chain([self.second.as_ref(), self.lobes.as_ref()])
            .flatten()
    }

    pub fn retire(self, gpu: &mut GpuDevice) -> Result<()> {
        for image in self.images() {
            gpu.retire(Resource::Image(image.image))?;
        }
        Ok(())
    }
}
