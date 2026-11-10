//! Scalar AT1 oracle, with independent integration and hardware-format stores.
use super::reference::{Image, half, sample};
use renderlab::{AtmosphereMedium, Vec3};
const PI: f32 = std::f32::consts::PI;
pub struct Model {
    pub medium: AtmosphereMedium,
    pub sun: Vec3,
    pub altitude: f32,
}
impl Model {
    fn medium(&self, h: f32) -> (Vec3, Vec3, Vec3, Vec3) {
        let m = self.medium;
        let dr = (-h.max(0.) * (1. / m.rayleigh_scale_height_km)).exp();
        let dm = (-h.max(0.) * (1. / m.mie_scale_height_km)).exp();
        let ozone = (1. - (h - m.ozone_peak_km).abs() / m.ozone_half_width_km).max(0.);
        let ray = Vec3::from_array(m.rayleigh_scattering) * dr;
        let mie = Vec3::from_array(m.mie_scattering) * dm;
        (
            ray + mie,
            ray + mie
                + Vec3::from_array(m.mie_absorption) * dm
                + Vec3::from_array(m.ozone_absorption) * ozone,
            ray,
            mie,
        )
    }
    fn sphere(&self, p: Vec3, d: Vec3, r: f32) -> f32 {
        let b = p.dot(d);
        let c = p.dot(p) - r * r;
        let disc = b * b - c;
        if disc < 0. {
            return -1.;
        }
        let s = disc.sqrt();
        if -b - s >= 0. {
            -b - s
        } else if -b + s >= 0. {
            -b + s
        } else {
            -1.
        }
    }
    fn unit_uv(&self, x: f32, n: f32) -> f32 {
        0.5 / n + x * (1. - 1. / n)
    }
    fn trans_sample(&self, image: &Image, r: f32, mu: f32) -> Vec3 {
        let bottom = self.medium.ground_radius_km;
        let top = self.medium.top_radius_km;
        if mu < -(1. - bottom * bottom / (r * r)).max(0.).sqrt() {
            return Vec3::ZERO;
        }
        let h = (top * top - bottom * bottom).max(0.).sqrt();
        let rho = (r * r - bottom * bottom).max(0.).sqrt();
        let disc = r * r * (mu * mu - 1.) + top * top;
        let d = (-r * mu + disc.max(0.).sqrt()).max(0.);
        let u = (d - (top - r)) / (rho + h - (top - r)).max(1e-6);
        let v = rho / h.max(1e-6);
        rgb(sample(
            image,
            [self.unit_uv(u, 256.), self.unit_uv(v, 64.) * 64. / 65.],
        ))
    }
    pub fn transmittance(&self) -> Image {
        let bottom = self.medium.ground_radius_km;
        let top = self.medium.top_radius_km;
        let h = (top * top - bottom * bottom).sqrt();
        let mut image = Image {
            size: [256, 65],
            pixels: vec![[0.; 4]; 256 * 65],
        };
        for y in 0..64 {
            for x in 0..256 {
                let xmu = ((x as f32 + 0.5) / 256. - 0.5 / 256.) / (1. - 1. / 256.);
                let xr = ((y as f32 + 0.5) / 64. - 0.5 / 64.) / (1. - 1. / 64.);
                let rho = h * xr;
                let r = (rho * rho + bottom * bottom).sqrt();
                let min = top - r;
                let max = rho + h;
                let d = min + xmu * (max - min);
                let mu = if d > 0. {
                    ((h * h - rho * rho - d * d) / (2. * r * d)).clamp(-1., 1.)
                } else {
                    1.
                };
                let p = Vec3::new(0., r, 0.);
                let dir = Vec3::new((1. - mu * mu).max(0.).sqrt(), mu, 0.);
                let t = self.sphere(p, dir, top).max(0.);
                let mut depth = Vec3::ZERO;
                for i in 0..40 {
                    let s = t * (i as f32 + 0.5) / 40.;
                    depth += self.medium((p + dir * s).length() - bottom).1 * (t / 40.);
                }
                image.pixels[y * 256 + x] = rgba_half((-depth).exp());
            }
        }
        image
    }
    pub fn multiscatter(&self, trans: &Image) -> Image {
        let bottom = self.medium.ground_radius_km;
        let top = self.medium.top_radius_km;
        let pixels = (0..32 * 32)
            .map(|pixel| {
                let uv = [
                    ((pixel % 32) as f32 + 0.5) / 32.,
                    ((pixel / 32) as f32 + 0.5) / 32.,
                ];
                let mu = uv[0] * 2. - 1.;
                let r = (bottom + uv[1] * (top - bottom)).clamp(bottom + 1e-3, top - 1e-3);
                let p = Vec3::new(0., r, 0.);
                let sun = Vec3::new((1. - mu * mu).max(0.).sqrt(), mu, 0.);
                let mut total_l = Vec3::ZERO;
                let mut total_f = Vec3::ZERO;
                for lane in 0..64 {
                    let a = ((lane % 8) as f32 + 0.5) / 8.;
                    let b = ((lane / 8) as f32 + 0.5) / 8.;
                    let ct = 1. - 2. * a;
                    let st = (1. - ct * ct).max(0.).sqrt();
                    let phi = 2. * PI * b;
                    let d = Vec3::new(st * phi.cos(), ct, st * phi.sin());
                    let ground = self.sphere(p, d, bottom);
                    let t = if ground > 0. {
                        ground
                    } else {
                        self.sphere(p, d, top).max(0.)
                    };
                    let dt = t / 20.;
                    let mut l = Vec3::ZERO;
                    let mut f = Vec3::ZERO;
                    let mut throughput = Vec3::ONE;
                    for i in 0..20 {
                        let q = p + d * (dt * (i as f32 + 0.5));
                        let rq = q.length();
                        let (scat, ext, _, _) = self.medium(rq - bottom);
                        let segment = (-ext * dt).exp();
                        let lit = if self.sphere(q, sun, bottom) > 0. {
                            0.
                        } else {
                            1.
                        };
                        let s = scat * self.trans_sample(trans, rq, (q / rq).dot(sun)) * lit
                            / (4. * PI);
                        let inv = Vec3::ONE / ext.max(Vec3::splat(1e-7));
                        l += throughput * (s - s * segment) * inv;
                        f += throughput * (scat - scat * segment) * inv;
                        throughput *= segment;
                    }
                    if ground > 0. {
                        let q = p + d * ground;
                        let mu = q.normalize().dot(sun);
                        l += throughput
                            * self.trans_sample(trans, bottom, mu)
                            * mu.max(0.)
                            * Vec3::from_array(self.medium.ground_albedo)
                            / PI;
                    }
                    total_l += l;
                    total_f += f;
                }
                rgba_half((total_l / 64.) / (Vec3::ONE - total_f / 64.).max(Vec3::splat(1e-3)))
            })
            .collect();
        Image {
            size: [32, 32],
            pixels,
        }
    }
    fn multi_sample(&self, image: &Image, r: f32, mu: f32) -> Vec3 {
        let u = (mu * 0.5 + 0.5).clamp(0., 1.);
        let v = ((r - self.medium.ground_radius_km)
            / (self.medium.top_radius_km - self.medium.ground_radius_km))
            .clamp(0., 1.);
        rgb(sample(image, [self.unit_uv(u, 32.), self.unit_uv(v, 32.)]))
    }
    fn sky_direction(&self, uv: [f32; 2]) -> Vec3 {
        let r = self.medium.ground_radius_km + self.altitude;
        let bottom = self.medium.ground_radius_km;
        let horizon = (r * r - bottom * bottom).max(0.).sqrt();
        let beta = (horizon / r).clamp(-1., 1.).acos();
        let zh = PI - beta;
        let zenith = if uv[1] < 0.5 {
            let c = 1. - 2. * uv[1];
            zh * (1. - c * c)
        } else {
            let c = 2. * uv[1] - 1.;
            zh + beta * c * c
        };
        let lv = -(uv[0] * uv[0] * 2. - 1.);
        let local = Vec3::new(
            zenith.sin() * lv,
            zenith.cos(),
            zenith.sin() * (1. - lv * lv).max(0.).sqrt(),
        );
        let h = glam::Vec2::new(self.sun.x, self.sun.z);
        let a = if h.length_squared() > 1e-12 {
            h.normalize()
        } else {
            glam::Vec2::X
        };
        Vec3::new(
            a.x * local.x - a.y * local.z,
            local.y,
            a.y * local.x + a.x * local.z,
        )
    }
    pub fn sky(&self, trans: &Image, multi: &Image) -> Image {
        let bottom = self.medium.ground_radius_km;
        let p = Vec3::new(0., bottom + self.altitude, 0.);
        let pixels = (0..192 * 108)
            .map(|pixel| {
                let uv = [
                    ((pixel % 192) as f32 + 0.5) / 192.,
                    ((pixel / 192) as f32 + 0.5) / 108.,
                ];
                let d = self.sky_direction(uv);
                let ground = self.sphere(p, d, bottom);
                let top = self.sphere(p, d, self.medium.top_radius_km);
                let mut l = Vec3::ZERO;
                let mut throughput = Vec3::ONE;
                if top > 0. {
                    let max = if ground > 0. { ground } else { top };
                    let mut previous = 0.;
                    let c = d.dot(self.sun);
                    let pr = 3. / (16. * PI) * (1. + c * c);
                    let g = self.medium.mie_g;
                    let pm = 3. / (8. * PI) * (1. - g * g) / (2. + g * g) * (1. + c * c)
                        / (1. + g * g - 2. * g * c).max(1e-6).powf(1.5);
                    for i in 0..30 {
                        let x = (i as f32 + 1.) / 30.;
                        let t = max * x * x;
                        let dt = t - previous;
                        let q = p + d * (previous + dt * 0.5);
                        previous = t;
                        let r = q.length();
                        let (scat, ext, ray, mie) = self.medium(r - bottom);
                        let segment = (-ext * dt).exp();
                        let mu = (q / r).dot(self.sun);
                        let lit = if self.sphere(q, self.sun, bottom) > 0. {
                            0.
                        } else {
                            1.
                        };
                        let s = (ray * pr + mie * pm) * self.trans_sample(trans, r, mu) * lit
                            + scat * self.multi_sample(multi, r, mu);
                        l += throughput * (s - s * segment) / ext.max(Vec3::splat(1e-7));
                        throughput *= segment;
                    }
                    if ground > 0. {
                        let q = p + d * ground;
                        let mu = q.normalize().dot(self.sun);
                        let irradiance = self.trans_sample(trans, bottom, mu) * mu.max(0.);
                        l += throughput * irradiance * Vec3::from_array(self.medium.ground_albedo)
                            / PI;
                    }
                }
                rgba_half(l)
            })
            .collect();
        Image {
            size: [192, 108],
            pixels,
        }
    }
    fn sky_uv(&self, d: Vec3) -> [f32; 2] {
        let bottom = self.medium.ground_radius_km;
        let r = bottom + self.altitude;
        let beta = ((r * r - bottom * bottom).max(0.).sqrt() / r)
            .clamp(-1., 1.)
            .acos();
        let zh = PI - beta;
        let zenith = d.y.clamp(-1., 1.).acos();
        let v = if zenith < zh {
            (1. - (1. - zenith / zh).max(0.).sqrt()) * 0.5
        } else {
            ((zenith - zh) / beta.max(1e-6)).clamp(0., 1.).sqrt() * 0.5 + 0.5
        };
        let s = glam::Vec2::new(self.sun.x, self.sun.z);
        let h = glam::Vec2::new(d.x, d.z);
        let lv = if s.length_squared() > 1e-12 && h.length_squared() > 1e-12 {
            s.normalize().dot(h.normalize())
        } else {
            1.
        };
        [(-lv * 0.5 + 0.5).clamp(0., 1.).sqrt(), v]
    }
    pub fn ambient(&self, sky: &Image) -> [f32; 4] {
        let mut total = Vec3::ZERO;
        for lane in 0..64 {
            let mut sum = Vec3::ZERO;
            for k in 0..4 {
                let i = lane * 4 + k;
                let a = ((i % 16) as f32 + 0.5) / 16.;
                let b = ((i / 16) as f32 + 0.5) / 16.;
                let ct = 1. - 2. * a;
                let st = (1. - ct * ct).max(0.).sqrt();
                let phi = 2. * PI * b;
                let d = Vec3::new(st * phi.cos(), ct, st * phi.sin());
                sum += rgb(sample(sky, self.sky_uv(d)));
            }
            total += sum;
        }
        rgba_half(total / 256.)
    }
}
fn rgb(p: [f32; 4]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}
fn rgba_half(v: Vec3) -> [f32; 4] {
    half([v.x, v.y, v.z, 1.])
}
