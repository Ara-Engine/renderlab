//! Redistributable S2-glossy screen normal distribution, prepared without a GPU.
use renderlab_asset::{Asset, Limits, Mip, Texture, TextureRole, encode};
use renderlab_cook::normal_map;
use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("expected output directory")?,
    );
    fs::create_dir_all(&root)?;
    let size = 512u32;
    let mut state = 0x6a09e667u32;
    let mut uniform = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (f64::from(state) + 0.5) / 4294967296.0
    };
    let mut normals = Vec::new();
    for _ in 0..size * size {
        let radius = (-2.0 * uniform().ln()).sqrt() * 0.035;
        let angle = uniform() * std::f64::consts::TAU;
        let (x, y) = (radius * angle.cos(), radius * angle.sin());
        let length = (1.0 + x * x + y * y).sqrt();
        normals.push([
            (x / length) as f32,
            (y / length) as f32,
            (1.0 / length) as f32,
        ]);
    }
    let prepared = normal_map::prepare_vectors(size, size, normals)?;
    let texture = Texture {
        role: TextureRole::NormalVarianceLinear,
        mips: prepared
            .mips()
            .iter()
            .map(|m| Mip {
                width: m.width(),
                height: m.height(),
                rgba8: m.rgba8(),
            })
            .collect(),
    };
    // Frozen pre-U1 recipe: normalize each level before constructing the next.
    let mut legacy = texture.clone();
    legacy.role = TextureRole::NormalLinear;
    legacy.mips.clear();
    let mut vectors = prepared.mips()[0].vectors().to_vec();
    let mut side = size;
    loop {
        let step = normal_map::prepare_vectors(side, side, vectors)?;
        let mut bytes = step.mips()[0].rgba8();
        for p in bytes.chunks_exact_mut(4) {
            p[3] = 255;
        }
        legacy.mips.push(Mip {
            width: side,
            height: side,
            rgba8: bytes,
        });
        if side == 1 {
            break;
        }
        vectors = step.mips()[1].vectors().to_vec();
        side /= 2;
    }
    fs::write(
        root.join("fixture.rla"),
        encode(
            &Asset {
                textures: vec![texture, legacy],
                ..Default::default()
            },
            Limits::default(),
        )?,
    )?;
    Ok(())
}
