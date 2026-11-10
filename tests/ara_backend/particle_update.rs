//! Scalar update oracle for the original particle kernel with unshadowed lighting.
use renderlab::{Mat4, Vec3};

/// Original source-layout inputs and the expected twelve-float particle records.
pub struct Fixture {
    /// `TpFrame`: three matrices followed by nine vec4 records, 336 bytes.
    pub frame: [u32; 84],
    /// `TpEmitter`: nine float vec4 records and raw u32 counts, 160 bytes.
    pub emitter: [u32; 40],
    /// `LightingUniform`: eye/count, constant environment/exposure and map control.
    pub lighting: [u32; 12],
    /// Three 80-byte light records; the middle point light must be ignored.
    pub lights: Vec<[u32; 20]>,
    /// Disabled `TpTransmission`, with its source matrix and zero layer count.
    pub transmission: [u32; 24],
    /// Twenty 48-byte source records; capacity is seventeen, leaving three sentinels.
    pub particles: Vec<[u32; 12]>,
    /// CPU results, including exact preservation of dead records and buffer padding.
    pub expected: Vec<[f32; 12]>,
}

fn vector(words: &[u32], offset: usize) -> Vec3 {
    Vec3::new(
        f32::from_bits(words[offset]),
        f32::from_bits(words[offset + 1]),
        f32::from_bits(words[offset + 2]),
    )
}

fn put(words: &mut [u32], offset: usize, values: &[f32]) {
    for (word, value) in words[offset..].iter_mut().zip(values) {
        *word = value.to_bits();
    }
}

/// PCG-style integer permutation, with explicit WGSL u32 modular arithmetic.
fn hash(value: u32) -> u32 {
    let state = value.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let permuted = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (permuted >> 22) ^ permuted
}

fn uniform(seed: &mut u32) -> f32 {
    *seed = hash(*seed);
    (*seed >> 8) as f32 / 16_777_216.
}

/// Inverse-CDF sphere sampling with three sequential random draws.
fn ball(seed: &mut u32) -> Vec3 {
    let z = 2. * uniform(seed) - 1.;
    let azimuth = uniform(seed) * std::f32::consts::TAU;
    let radius = uniform(seed).powf(1. / 3.);
    let equator = (1. - z * z).max(0.).sqrt();
    Vec3::new(equator * azimuth.cos(), equator * azimuth.sin(), z) * radius
}

/// Analytic open-sky result: half the directional sum plus constant-sky irradiance.
fn incident(lighting: &[u32; 12], lights: &[[u32; 20]]) -> Vec3 {
    assert_eq!(
        f32::from_bits(lighting[11]),
        0.,
        "constant environment required"
    );
    assert!(lighting[3] as usize <= lights.len());
    let mut directional = Vec3::ZERO;
    for light in &lights[..lighting[3] as usize] {
        if light[12] == 0 {
            assert_eq!(
                light[14], 0,
                "oracle requires unshadowed directional lights"
            );
            directional += vector(light, 8);
        }
    }
    directional * 0.5 + vector(lighting, 4) * std::f32::consts::PI
}

/// Update particle records, preserving every word not written by the original kernel.
///
/// This fixture oracle supports constant environments and unshadowed directional
/// irradiance, with `tp.sun.w == 0` and transmission disabled. Other light kinds
/// are skipped, as in the kernel. Particle lighting is unexposed: neither exposure
/// nor `tp.sun_lux` scales the result. Records beyond capacity remain untouched.
pub fn update(
    frame: &[u32; 84],
    emitter: &[u32; 40],
    particles: &[[u32; 12]],
    lighting: &[u32; 12],
    lights: &[[u32; 20]],
) -> Vec<[f32; 12]> {
    let capacity = emitter[32];
    assert!(capacity > 0 && capacity as usize <= particles.len());
    assert_eq!(
        f32::from_bits(frame[71]),
        0.,
        "sun shadow map must be disabled"
    );
    let dt = f32::from_bits(frame[80]);
    let first = emitter[33];
    let spawned = emitter[34];
    let emitter_seed = emitter[35];
    let irradiance = incident(lighting, lights);
    let mut output: Vec<_> = particles
        .iter()
        .map(|particle| particle.map(f32::from_bits))
        .collect();
    for (slot, particle) in output.iter_mut().take(capacity as usize).enumerate() {
        let spawn_index = (slot as u32)
            .wrapping_add(capacity)
            .wrapping_sub(first % capacity)
            % capacity;
        if spawn_index < spawned {
            let mut seed = hash(first.wrapping_add(spawn_index) ^ hash(emitter_seed));
            let fraction = uniform(&mut seed);
            let lifetime = f32::from_bits(emitter[28]) * (1. - fraction)
                + f32::from_bits(emitter[29]) * fraction;
            let age =
                dt * ((spawned - 1 - spawn_index) as f32 + uniform(&mut seed)) / spawned as f32;
            let velocity = vector(emitter, 4) + ball(&mut seed) * f32::from_bits(emitter[7]);
            let position =
                vector(emitter, 0) + ball(&mut seed) * f32::from_bits(emitter[3]) + velocity * age;
            *particle = [
                position.x, position.y, position.z, age, velocity.x, velocity.y, velocity.z,
                lifetime, 0., 0., 0., 0.,
            ];
        } else if particle[3] < particle[7] {
            let attenuation = (-f32::from_bits(emitter[11]) * dt).exp();
            let velocity = Vec3::new(particle[4], particle[5], particle[6]) * attenuation
                + vector(emitter, 8) * dt;
            let position = Vec3::new(particle[0], particle[1], particle[2]) + velocity * dt;
            particle[..3].copy_from_slice(&position.to_array());
            particle[3] += dt;
            particle[4..7].copy_from_slice(&velocity.to_array());
        }
        // A particle that dies during this step retains its old irradiance and W.
        if particle[3] < particle[7] {
            particle[8..11].copy_from_slice(&irradiance.to_array());
            particle[11] = 0.;
        }
    }
    output
}

/// Wrapped spawning, varied liveness and exact-boundary expiry in one partial group.
///
/// Spawn slots are 16,0,1,2,3, with u32 spawn-counter wrap after the second birth.
/// Slot 4 integrates alive; 5 dies during integration; 6 is already dead; 7 has
/// never lived; 8 expires exactly at lifetime; 9 has a sub-step lifetime. Further
/// alive/dead records vary velocity and age, and slots 17..20 are untouched padding.
pub fn fixture() -> Fixture {
    let mut frame = [0; 84];
    let identity = Mat4::IDENTITY.to_cols_array();
    for offset in [0, 16, 32] {
        put(&mut frame, offset, &identity);
    }
    put(&mut frame, 48, &[0., 0., 0., 1.75]);
    put(&mut frame, 52, &[0., 0., -1., 0.]);
    put(&mut frame, 56, &[1., 0., 0., 0.]);
    put(&mut frame, 60, &[0., 1., 0., 0.]);
    put(&mut frame, 64, &[32., 24., 1. / 32., 1. / 24.]);
    let direction = Vec3::new(-0.55, 0.25, 0.8).normalize();
    put(&mut frame, 68, &[direction.x, direction.y, direction.z, 0.]);
    put(&mut frame, 72, &[37., 19., 11., 0.25]);
    frame[76..80].copy_from_slice(&[0, 0, 32, 24]);
    put(&mut frame, 80, &[0.125, 0.001, 1., 0.]);

    let mut emitter = [0; 40];
    put(&mut emitter, 0, &[0.5, 1.25, -2., 0.65]);
    put(&mut emitter, 4, &[1., 0.75, -0.5, 0.8]);
    put(&mut emitter, 8, &[0.4, -9.5, 0.25, 0.75]);
    put(&mut emitter, 12, &[0.8, 0.6, 0.3, 0.9]);
    put(&mut emitter, 16, &[0.15, 0.3, 0.7, 0.]);
    put(&mut emitter, 20, &[0.1, 0.05, 0.02, 0.25]);
    put(&mut emitter, 24, &[0., 0., 0., 0.7]);
    put(&mut emitter, 28, &[0.35, 0.9, 0.05, 0.2]);
    emitter[32..36].copy_from_slice(&[17, 0xffff_fffe, 5, 0x1357_9bdf]);
    put(&mut emitter, 36, &[0.4, 32., 0., 0.]);

    let mut lighting = [0; 12];
    lighting[3] = 3;
    put(&mut lighting, 4, &[0.05, 0.04, 0.03, 1.75]);
    put(&mut lighting, 8, &[1., 0., 1., 0.]);
    let directional = |direction: Vec3, rgb: [f32; 3]| {
        let mut record = [0; 20];
        put(&mut record, 4, &(-direction).to_array());
        put(&mut record, 8, &rgb);
        record
    };
    let mut point = [0; 20];
    put(&mut point, 0, &[0.25, 1., -0.5, 4.]);
    put(&mut point, 8, &[25., 13., 9.]);
    point[12] = 1;
    let lights = vec![
        directional(direction, [2., 1., 0.5]),
        point,
        directional(Vec3::new(0.2, 0.9, -0.3).normalize(), [0.3, 0.5, 0.8]),
    ];
    let mut transmission = [0; 24];
    put(&mut transmission, 0, &identity);
    put(
        &mut transmission,
        16,
        &[direction.x, direction.y, direction.z, 0.],
    );

    let mut particles: Vec<_> = (0..20)
        .map(|slot| {
            let s = slot as f32;
            [
                0.1 * s,
                0.3 + 0.05 * s,
                -1. - 0.2 * s,
                0.2,
                -0.7 + 0.04 * s,
                0.2 - 0.03 * s,
                0.6,
                1.2,
                8. + s,
                4. + 0.5 * s,
                2. + 0.25 * s,
                10. + 0.125 * s,
            ]
        })
        .collect();
    for (slot, age, lifetime) in [
        (4, 0.1, 1.2),
        (5, 0.95, 1.),
        (6, 1.5, 1.),
        (7, 0., 0.),
        (8, 0.875, 1.),
        (9, 0., 0.05),
        (10, 0.25, 1.),
        (11, 0.4, 0.4),
        (12, 0.4, 2.),
        (13, 0.8, 2.),
        (14, 2., 0.),
        (15, 0., 4.),
    ] {
        particles[slot][3] = age;
        particles[slot][7] = lifetime;
    }
    particles[10][4..7].fill(0.);
    let particles: Vec<_> = particles
        .into_iter()
        .map(|row| row.map(f32::to_bits))
        .collect();
    let expected = update(&frame, &emitter, &particles, &lighting, &lights);
    Fixture {
        frame,
        emitter,
        lighting,
        lights,
        transmission,
        particles,
        expected,
    }
}
