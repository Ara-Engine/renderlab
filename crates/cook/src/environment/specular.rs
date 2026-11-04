use super::*;

pub(super) fn sample_counts(total: u32) -> (u32, u32) {
    (total - total / 2, total / 2)
}

// Each CDF bin is an equirectangular pixel's spherical rectangle, uniform in solid angle.
// The proposal need not match bilinear radiance exactly, but its queried PDF must match sampling.
struct RadianceProposal {
    cdf: Vec<f64>,
    baseline: [f64; 3],
    total: f64,
    width: u32,
    height: u32,
}

impl RadianceProposal {
    fn new(input: &LinearEnvironment) -> Result<Self> {
        let mut baseline = [f64::MAX; 3];
        for p in &input.pixels {
            for c in 0..3 {
                baseline[c] = baseline[c].min(f64::from(p[c]));
            }
        }
        let mut proposal = Self {
            cdf: Vec::new(),
            baseline,
            total: 0.0,
            width: input.width,
            height: input.height,
        };
        proposal
            .cdf
            .try_reserve_exact(input.pixels.len())
            .map_err(|_| Error("environment radiance CDF allocation".into()))?;
        for y in 0..input.height {
            let omega = proposal.solid_angle(y);
            for x in 0..input.width {
                let p =
                    input.pixels[(u64::from(y) * u64::from(input.width) + u64::from(x)) as usize];
                let luminance = [0.2126, 0.7152, 0.0722]
                    .into_iter()
                    .enumerate()
                    .map(|(c, w)| w * (f64::from(p[c]) - baseline[c]))
                    .sum::<f64>();
                // Broaden faint-tail coverage. The square root changes only proposal mass.
                proposal.total += luminance.sqrt() * omega;
                proposal.cdf.push(proposal.total);
            }
        }
        Ok(proposal)
    }

    fn solid_angle(&self, y: u32) -> f64 {
        let step = PI / f64::from(self.height);
        TAU / f64::from(self.width) * 2.0 * ((f64::from(y) + 0.5) * step).sin() * (step * 0.5).sin()
    }

    fn bin_pdf(&self, index: usize, y: u32) -> f64 {
        let previous = if index == 0 { 0.0 } else { self.cdf[index - 1] };
        (self.cdf[index] - previous) / (self.total * self.solid_angle(y))
    }

    fn pdf(&self, d: [f64; 3]) -> f64 {
        let u = d[2].atan2(d[0]) / TAU + 0.5;
        let v = d[1].clamp(-1.0, 1.0).acos() / PI;
        let x = (u * f64::from(self.width)).floor() as u32 % self.width;
        let y = ((v * f64::from(self.height)).floor() as u32).min(self.height - 1);
        self.bin_pdf(
            (u64::from(y) * u64::from(self.width) + u64::from(x)) as usize,
            y,
        )
    }

    fn sample(&self, index: u32, count: u32) -> ([f64; 3], f64) {
        let target = (f64::from(index) + 0.5) / f64::from(count) * self.total;
        let bin = self
            .cdf
            .partition_point(|v| *v <= target)
            .min(self.cdf.len() - 1);
        let x = bin % self.width as usize;
        let y = (bin / self.width as usize) as u32;
        let v = (f64::from(index.reverse_bits()) + 0.5) / 4294967296.0;
        let theta0 = PI * f64::from(y) / f64::from(self.height);
        let theta1 = PI * f64::from(y + 1) / f64::from(self.height);
        let cos_theta = theta0.cos() * (1.0 - v) + theta1.cos() * v;
        let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
        let mut digits = index + 1;
        let mut factor = 1.0 / 3.0;
        let mut u = 0.0;
        while digits > 0 {
            u += f64::from(digits % 3) * factor;
            digits /= 3;
            factor /= 3.0;
        }
        let phi = TAU * ((x as f64 + u) / f64::from(self.width) - 0.5);
        (
            [sin_theta * phi.cos(), cos_theta, sin_theta * phi.sin()],
            self.bin_pdf(bin, y),
        )
    }
}

struct RadianceSample {
    direction: [f64; 3],
    value: [f64; 3],
    pdf: f64,
}

pub(super) struct Quadrature<'a> {
    input: &'a LinearEnvironment,
    proposal: RadianceProposal,
    radiance: Vec<RadianceSample>,
    ggx_count: u32,
}

impl<'a> Quadrature<'a> {
    pub(super) fn new(input: &'a LinearEnvironment, count: u32) -> Result<Self> {
        let proposal = RadianceProposal::new(input)?;
        let (ggx_count, hdr_count) = sample_counts(count);
        let mut radiance = Vec::new();
        if proposal.total > 0.0 {
            radiance
                .try_reserve_exact(hdr_count as usize)
                .map_err(|_| Error("environment radiance samples allocation".into()))?;
            for i in 0..hdr_count {
                let (direction, pdf) = proposal.sample(i, hdr_count);
                radiance.push(RadianceSample {
                    direction,
                    value: input.sample(direction),
                    pdf,
                });
            }
        }
        Ok(Self {
            input,
            proposal,
            radiance,
            ggx_count,
        })
    }

    pub(super) fn integrate(
        &self,
        n: [f64; 3],
        roughness: f64,
        samples: &[([f64; 3], f64)],
    ) -> [f64; 3] {
        if self.proposal.total == 0.0 {
            return self.proposal.baseline;
        }
        let up = if n[1].abs() < 0.99 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let t = cross(up, n);
        let length = dot(t, t).sqrt();
        let t = t.map(|v| v / length);
        let b = cross(n, t);
        let ng = f64::from(self.ggx_count);
        let nh = self.radiance.len() as f64;
        let mut rgb = [0.0; 3];
        let mut accumulate = |value: [f64; 3], cosine: f64, ggx_pdf: f64, hdr_pdf: f64| {
            let weight = ggx_pdf * cosine.max(0.0) / (ng * ggx_pdf + nh * hdr_pdf);
            for c in 0..3 {
                rgb[c] += (value[c] - self.proposal.baseline[c]).max(0.0) * weight;
            }
        };
        for &(d, ggx_pdf) in samples {
            if d[2] <= 0.0 {
                continue;
            }
            let world = std::array::from_fn(|i| t[i] * d[0] + b[i] * d[1] + n[i] * d[2]);
            accumulate(
                self.input.sample(world),
                d[2],
                ggx_pdf,
                self.proposal.pdf(world),
            );
        }
        for sample in &self.radiance {
            let cosine = dot(n, sample.direction).clamp(-1.0, 1.0);
            if cosine <= 0.0 {
                continue;
            }
            accumulate(sample.value, cosine, ggx_pdf(cosine, roughness), sample.pdf);
        }
        let normalization = kernel_integral(roughness);
        std::array::from_fn(|c| self.proposal.baseline[c] + rgb[c] / normalization)
    }
}

pub(super) fn ggx_samples(count: u32, roughness: f64) -> Result<Vec<([f64; 3], f64)>> {
    let mut table = Vec::new();
    table
        .try_reserve_exact(count as usize)
        .map_err(|_| Error("environment GGX samples allocation".into()))?;
    let a = (roughness * roughness).max(asset::GGX_ALPHA_FLOOR).powi(2);
    for i in 0..count {
        let phi = TAU * (f64::from(i) + 0.5) / f64::from(count);
        let v = f64::from(i.reverse_bits()) / 4294967296.0;
        let ch = ((1.0 - v) / (1.0 + (a - 1.0) * v)).sqrt();
        let sh = (1.0 - ch * ch).max(0.0).sqrt();
        let d = [
            2.0 * ch * sh * phi.cos(),
            2.0 * ch * sh * phi.sin(),
            2.0 * ch * ch - 1.0,
        ];
        table.push((d, ggx_pdf(d[2], roughness)));
    }
    Ok(table)
}

fn ggx_pdf(cosine: f64, roughness: f64) -> f64 {
    let a = (roughness * roughness).max(asset::GGX_ALPHA_FLOOR).powi(2);
    a / (4.0 * PI * (a + (1.0 - a) * (1.0 - cosine) * 0.5).powi(2))
}

// Integral of D(H)*max(NoL,0)/4 for N=V. Use a series near a=alpha^2=1 to avoid cancellation.
fn kernel_integral(roughness: f64) -> f64 {
    let a = (roughness * roughness).max(asset::GGX_ALPHA_FLOOR).powi(2);
    if a > 0.9 {
        let x = (1.0 - a) / (1.0 + a);
        let mut power = 1.0;
        let mut sum = 0.0;
        for k in 0..16 {
            sum += f64::from(k + 1) / f64::from(k + 2) * power;
            power *= x;
        }
        2.0 * a / (1.0 + a).powi(2) * sum
    } else {
        (1.0 - a + 2.0 * a * (2.0 * a / (1.0 + a)).ln()) / (1.0 - a).powi(2)
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The compute ABI rounds only after accepted f64 proposal/sample/frame construction.
#[cfg(feature = "gpu-environment")]
pub(super) fn gpu_tables(input: &LinearEnvironment) -> Result<[Vec<u8>; 4]> {
    let q = Quadrature::new(input, 16384)?;
    let mut source = Vec::with_capacity(input.pixels.len() * 16);
    for (i, p) in input.pixels.iter().enumerate() {
        let pdf = if q.proposal.total == 0.0 {
            0.0
        } else {
            q.proposal.bin_pdf(i, (i / input.width as usize) as u32)
        };
        gpu_record(
            &mut source,
            [f64::from(p[0]), f64::from(p[1]), f64::from(p[2]), pdf],
        )?;
    }
    let mut hdr = Vec::with_capacity(8192 * 32);
    for sample in &q.radiance {
        gpu_record(
            &mut hdr,
            [
                sample.direction[0],
                sample.direction[1],
                sample.direction[2],
                sample.pdf,
            ],
        )?;
        gpu_record(
            &mut hdr,
            [sample.value[0], sample.value[1], sample.value[2], 0.0],
        )?;
    }
    // Constant controls use the same kernel and full counts, with zero residual on both proposals.
    if hdr.is_empty() {
        for _ in 0..8192 {
            gpu_record(&mut hdr, [0.0, 1.0, 0.0, 1.0])?;
            gpu_record(
                &mut hdr,
                [
                    q.proposal.baseline[0],
                    q.proposal.baseline[1],
                    q.proposal.baseline[2],
                    0.0,
                ],
            )?;
        }
    }
    let mut ggx = Vec::with_capacity(64 * 8192 * 16);
    for r in asset::roughness_knots().into_iter().skip(1) {
        for (d, pdf) in ggx_samples(8192, f64::from(r))? {
            gpu_record(&mut ggx, [d[0], d[1], d[2], pdf])?;
        }
    }
    let mut config = Vec::with_capacity(64);
    gpu_record(
        &mut config,
        [
            q.proposal.baseline[0],
            q.proposal.baseline[1],
            q.proposal.baseline[2],
            0.0,
        ],
    )?;
    for row in [0, input.height as usize - 1] {
        let mut mean = [0.0; 3];
        for p in &input.pixels[row * input.width as usize..(row + 1) * input.width as usize] {
            for c in 0..3 {
                mean[c] += f64::from(p[c]) / f64::from(input.width);
            }
        }
        gpu_record(&mut config, [mean[0], mean[1], mean[2], 0.0])?;
    }
    gpu_record(
        &mut config,
        [f64::from(input.width), f64::from(input.height), 0.0, 0.0],
    )?;
    Ok([source, hdr, ggx, config])
}

#[cfg(feature = "gpu-environment")]
pub(super) fn gpu_record(out: &mut Vec<u8>, values: impl IntoIterator<Item = f64>) -> Result<()> {
    for value in values {
        let rounded = value as f32;
        if !rounded.is_finite() || (value != 0.0 && rounded == 0.0) {
            return Err(Error(
                "environment GPU input outside f32 range; no CPU fallback".into(),
            ));
        }
        out.extend_from_slice(&rounded.to_le_bytes());
    }
    Ok(())
}

#[cfg(feature = "gpu-environment")]
pub(super) fn gpu_query(out: &mut Vec<u8>, n: [f64; 3], level: u32) -> Result<()> {
    let r = f64::from(asset::roughness_knots()[level as usize]);
    let up = if n[1].abs() < 0.99 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t = cross(up, n);
    let length = dot(t, t).sqrt();
    let t = t.map(|v| v / length);
    let b = cross(n, t);
    gpu_record(
        out,
        [
            n[0],
            n[1],
            n[2],
            1.0 / kernel_integral(r),
            t[0],
            t[1],
            t[2],
            (r * r).max(asset::GGX_ALPHA_FLOOR).powi(2),
            b[0],
            b[1],
            b[2],
            f64::from((level - 1) * 8192),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radiance_proposal_uses_solid_angle_and_queries_the_sampled_pdf() {
        let input = LinearEnvironment::new(
            4,
            3,
            vec![
                [10.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], [10.0; 3], [0.0; 3], [0.0; 3],
                [0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3],
            ],
            CookLimits::default(),
        )
        .unwrap();
        let proposal = RadianceProposal::new(&input).unwrap();
        let pole_omega = TAU / 4.0 * (1.0 - (PI / 3.0).cos());
        let equator_omega = TAU / 4.0 * ((PI / 3.0).cos() - (2.0 * PI / 3.0).cos());
        assert!((proposal.total - 10.0_f64.sqrt() * (pole_omega + equator_omega)).abs() < 1e-12);
        let expected_pdf = 1.0 / (pole_omega + equator_omega);
        let mut polar_samples = 0;
        for i in 0..4096 {
            let (d, pdf) = proposal.sample(i, 4096);
            assert!((pdf - expected_pdf).abs() < 1e-12);
            assert!((pdf - proposal.pdf(d)).abs() < 1e-12);
            polar_samples += usize::from(d[1] > (PI / 3.0).cos());
        }
        assert!((polar_samples as f64 / 4096.0 - 1.0 / 3.0).abs() < 1.0 / 4096.0);
    }

    #[test]
    fn square_root_proposal_broadens_residual_mass_without_changing_radiance() {
        let input = LinearEnvironment::new(
            4,
            1,
            vec![[2.0; 3], [3.0; 3], [6.0; 3], [11.0; 3]],
            CookLimits::default(),
        )
        .unwrap();
        let proposal = RadianceProposal::new(&input).unwrap();
        assert_eq!(proposal.baseline, [2.0; 3]);
        assert!((proposal.total - 6.0 * PI).abs() < 1e-12);
        for (index, mass) in [0.0, 1.0, 2.0, 3.0].into_iter().enumerate() {
            assert!((proposal.bin_pdf(index, 0) - mass / (6.0 * PI)).abs() < 1e-12);
        }
        for i in 0..128 {
            let (direction, pdf) = proposal.sample(i, 128);
            assert!((pdf - proposal.pdf(direction)).abs() < 1e-12);
        }
        let q = Quadrature::new(&input, 17).unwrap();
        for sample in &q.radiance {
            assert_eq!(sample.value, input.sample(sample.direction));
        }
    }

    #[test]
    fn represented_cdf_mass_and_extreme_source_arithmetic_remain_finite() {
        let input = LinearEnvironment::new(
            4,
            1,
            vec![[0.0; 3], [f32::MAX; 3], [f32::from_bits(1); 3], [1.0; 3]],
            CookLimits::default(),
        )
        .unwrap();
        let q = Quadrature::new(&input, 128).unwrap();
        assert!(q.proposal.total.is_finite());
        // These positive residuals disappear when accumulated after the large bin in f64.
        assert_eq!(q.proposal.bin_pdf(2, 0), 0.0);
        assert_eq!(q.proposal.bin_pdf(3, 0), 0.0);
        let mut integrated_pdf = 0.0;
        for i in 0..4 {
            integrated_pdf += q.proposal.bin_pdf(i, 0) * PI;
        }
        assert!((integrated_pdf - 1.0).abs() < 1e-12);
        for sample in &q.radiance {
            assert!(sample.pdf.is_finite() && sample.pdf > 0.0);
            assert!((sample.pdf - q.proposal.pdf(sample.direction)).abs() < 1e-12);
        }
        for roughness in [0.0, 0.5, 0.999999, 1.0] {
            let samples = ggx_samples(64, roughness).unwrap();
            for n in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]] {
                assert!(
                    q.integrate(n, roughness, &samples)
                        .into_iter()
                        .all(|v| v.is_finite() && v >= 0.0)
                );
            }
        }
    }

    #[test]
    fn exact_kernel_normalization_and_control_preserve_constant_dynamic_range() {
        assert_eq!(kernel_integral(1.0), 0.25);
        assert!((kernel_integral(0.999999) - 0.25).abs() < 0.00001);
        assert!((kernel_integral(0.0) - 1.0).abs() < 1e-6);
        let input =
            LinearEnvironment::new(4, 2, vec![[1e-20, 7.0, 1e30]; 8], CookLimits::default())
                .unwrap();
        for count in [1, 2, 17, 16384] {
            let q = Quadrature::new(&input, count).unwrap();
            assert_eq!(q.proposal.total, 0.0);
            assert!(q.radiance.is_empty());
            for r in [0.0, 0.05, 0.5, 0.99, 1.0] {
                let samples = ggx_samples(sample_counts(count).0, r).unwrap();
                assert_eq!(
                    q.integrate([0.0, 1.0, 0.0], r, &samples),
                    input.pixels[0].map(f64::from)
                );
            }
        }
    }

    #[test]
    fn analytic_kernel_normalization_matches_independent_cosine_quadrature() {
        let transition = 0.9_f64.powf(0.25);
        for roughness in [0.5, transition - 1e-9, transition + 1e-9, 0.999999, 1.0] {
            let a = roughness.powi(4);
            let count = 8192;
            let mut reference = 0.0;
            for i in 0..count {
                let cosine = (f64::from(i) + 0.5) / f64::from(count);
                let half_cosine_squared = (1.0 + cosine) / 2.0;
                let denominator = 1.0 + (a - 1.0) * half_cosine_squared;
                reference += a * cosine / (2.0 * denominator * denominator * f64::from(count));
            }
            assert!((kernel_integral(roughness) - reference).abs() < 1e-7);
        }
    }

    #[test]
    fn changing_source_scale_preserves_linear_physical_radiance() {
        let pixels = [[0.0; 3], [1.0; 3], [4.0; 3], [9.0; 3]];
        let limits = CookLimits::default();
        let input = LinearEnvironment::new(4, 1, pixels.to_vec(), limits).unwrap();
        let samples = ggx_samples(128, 0.5).unwrap();
        let q = Quadrature::new(&input, 256).unwrap();
        let normal = [1.0, 0.0, 0.0];
        let reference = q.integrate(normal, 0.5, &samples);
        for scale in [1e-30_f32, 1e30] {
            let input = LinearEnvironment::new(
                4,
                1,
                pixels.into_iter().map(|p| p.map(|v| v * scale)).collect(),
                limits,
            )
            .unwrap();
            let q = Quadrature::new(&input, 256).unwrap();
            let actual = q.integrate(normal, 0.5, &samples);
            for (value, expected) in actual.into_iter().zip(reference) {
                assert!((value / f64::from(scale) - expected).abs() <= 1e-6 * expected);
            }
        }
    }

    #[test]
    fn proposal_buffers_and_preprocessing_are_admitted_before_allocation() {
        let input =
            LinearEnvironment::new(16, 8, vec![[1.0; 3]; 128], CookLimits::default()).unwrap();
        let settings = PreparationSettings {
            diffuse_size: 1,
            specular_size: 2,
            diffuse_samples: 4,
            specular_samples: 17,
        };
        let limits = CookLimits::default();
        let cost = estimate_cost(&input, settings, limits).unwrap();
        assert_eq!(
            cost.owned_working_bytes,
            128 * 12 + 36 * 12 + 128 * 8 + 8 * 56 + 9 * 32
        );
        assert_eq!(
            cost.sample_evaluations,
            6 * 4 + 6 * 17 + 24 + 4 + 128 * 2 + 8
        );
        assert!(
            prefilter_ggx_radiance(
                &input,
                settings,
                1.0,
                CookLimits {
                    max_working_bytes: cost.owned_working_bytes,
                    max_sample_evaluations: cost.sample_evaluations,
                    ..limits
                }
            )
            .is_ok()
        );
        assert!(
            prefilter_ggx_radiance(
                &input,
                settings,
                1.0,
                CookLimits {
                    max_working_bytes: cost.owned_working_bytes - 1,
                    ..limits
                }
            )
            .is_err()
        );
        assert!(
            prefilter_ggx_radiance(
                &input,
                settings,
                1.0,
                CookLimits {
                    max_sample_evaluations: cost.sample_evaluations - 1,
                    ..limits
                }
            )
            .is_err()
        );
    }

    fn sun_sky_source() -> LinearEnvironment {
        let pixels = (0..512)
            .flat_map(|y| (0..1024).map(move |x| (x, y)))
            .map(|(x, y)| {
                crate::environment_fixture::sun_sky(crate::environment_fixture::direction(
                    x, y, 1024, 512,
                ))
                .map(|v| v as f32)
            })
            .collect();
        LinearEnvironment::new(1024, 512, pixels, CookLimits::default()).unwrap()
    }

    #[test]
    fn grazing_f32_source_matches_the_unchanged_inherited_reference_gate() {
        let input = sun_sky_source();
        let q = Quadrature::new(&input, 16384).unwrap();
        let n = [0.5, 0.5, -1.0].map(|v| v / 1.5_f64.sqrt());
        let samples = ggx_samples(8192, 0.5).unwrap();
        let actual = q.integrate(n, 0.5, &samples);
        // Original independent 512x1024 angular reference from environment_preparation.
        let reference = [
            0.027871343950709965,
            0.04629707516056865,
            0.10393567197535857,
        ];
        for (value, expected) in actual.into_iter().zip(reference) {
            assert!(
                (value - expected).abs() <= 0.04 * expected + 0.0005,
                "actual={actual:?} reference={reference:?}"
            );
        }
        eprintln!("inherited f32 grazing RGB={actual:?} reference={reference:?}");
    }

    #[test]
    #[ignore = "requires the owned HDR and independent grazing neighborhood JSON"]
    fn grazing_sources_match_independent_neighborhood_references() {
        let reference = std::fs::read(
            std::env::var_os("RENDERLAB_ENVIRONMENT_GRAZING_REFERENCE")
                .expect("grazing reference JSON path"),
        )
        .unwrap();
        let reference: serde_json::Value = serde_json::from_slice(&reference).unwrap();
        let hdr =
            std::fs::read(std::env::var_os("RENDERLAB_ENVIRONMENT_HDR").expect("owned HDR path"))
                .unwrap();
        let hash = Sha256::digest(&hdr)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(reference["input_sha256"].as_str().unwrap(), hash);
        let hdr = crate::environment_hdr::decode_rec709_hdr(&hdr, CookLimits::default()).unwrap();
        let f32_source = sun_sky_source();
        let samples = ggx_samples(8192, 0.5).unwrap();
        for (source, input, absolute) in [("hdr", &hdr, 0.001), ("f32", &f32_source, 0.0005)] {
            let q = Quadrature::new(input, 16384).unwrap();
            let rows = reference[source].as_array().unwrap();
            assert_eq!(rows.len(), 5);
            let mut max_allowance = 0.0_f64;
            for row in rows {
                assert_eq!(row["reference_converged"].as_bool(), Some(true));
                let n = std::array::from_fn(|c| row["direction"][c].as_f64().unwrap());
                let actual = q.integrate(n, 0.5, &samples);
                for (c, value) in actual.into_iter().enumerate() {
                    let expected = row["reference_rgb"][c].as_f64().unwrap();
                    let fraction = (value - expected).abs() / (0.04 * expected.abs() + absolute);
                    max_allowance = max_allowance.max(fraction);
                    assert!(
                        fraction <= 1.0,
                        "{source} n={n:?} channel={c} actual={value} expected={expected}"
                    );
                }
            }
            eprintln!(
                "{source} grazing neighborhood, 5 directions, max allowance fraction={max_allowance}"
            );
        }
    }

    #[test]
    #[ignore = "requires the owned HDR and independently converged reference JSON"]
    fn actual_hdr_registered_integration() {
        let hdr =
            std::fs::read(std::env::var_os("RENDERLAB_ENVIRONMENT_HDR").expect("owned HDR path"))
                .unwrap();
        let reference = std::fs::read(
            std::env::var_os("RENDERLAB_ENVIRONMENT_REFERENCE").expect("reference JSON path"),
        )
        .unwrap();
        let reference: serde_json::Value = serde_json::from_slice(&reference).unwrap();
        let hash = Sha256::digest(&hdr)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(reference["input_sha256"].as_str().unwrap(), hash);
        let input = crate::environment_hdr::decode_rec709_hdr(&hdr, CookLimits::default()).unwrap();
        let q = Quadrature::new(&input, 16384).unwrap();
        let rows = reference["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 490);
        let mut max_allowance = 0.0_f64;
        for row in rows {
            assert_eq!(row["converged"].as_bool(), Some(true));
            let n = std::array::from_fn(|c| row["direction"][c].as_f64().unwrap());
            let r = row["roughness"].as_f64().unwrap();
            let samples = ggx_samples(8192, r).unwrap();
            let actual = q.integrate(n, r, &samples);
            for (c, value) in actual.into_iter().enumerate() {
                let expected = row["reference_rgb"][c].as_f64().unwrap();
                let fraction = (value - expected).abs() / (0.04 * expected.abs() + 0.001);
                max_allowance = max_allowance.max(fraction);
                assert!(
                    fraction <= 1.0,
                    "r={r} n={n:?} channel={c} actual={value} expected={expected}"
                );
            }
            if r == 0.05 && row["direction_index"].as_u64() == Some(30) {
                eprintln!(
                    "selected actual HDR r={r} n={n:?} RGB={actual:?} reference={}",
                    row["reference_rgb"]
                );
            }
        }
        eprintln!(
            "actual HDR: {} converged cases, 1470 channels, 16384 total samples (8192 GGX + 8192 HDR), max allowance fraction={max_allowance}",
            rows.len()
        );
    }
}
