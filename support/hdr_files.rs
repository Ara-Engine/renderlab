//! Tool-only evidence writer; the library exposes asynchronous bytes and identities.
use renderlab::{CaptureInfo, SampleCount, decode_f16};
use std::{fs, io::Write, path::Path};

pub fn write(
    prefix: &Path,
    info: CaptureInfo,
    bytes: &[u8],
    identities: serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    if info.format != renderlab::wgpu::TextureFormat::Rgba16Float
        || bytes.len() != info.width as usize * info.height as usize * 8
    {
        return Err("HDR evidence requires tightly packed RGBA16F".into());
    }
    if let Some(parent) = prefix.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(prefix.with_extension("rgba16f"), bytes)?;
    let values: Vec<f32> = bytes
        .chunks_exact(2)
        .map(|b| decode_f16(u16::from_le_bytes(b.try_into().unwrap())))
        .collect();
    let mut saturated = 0;
    let mut saturated_channels = 0;
    let mut nonfinite = 0;
    let mut rgb = Vec::with_capacity(info.width as usize * info.height as usize * 3);
    for p in values.chunks_exact(4) {
        saturated += usize::from(p[..3].iter().any(|v| *v >= 65504.));
        saturated_channels += p[..3].iter().filter(|v| **v >= 65504.).count();
        nonfinite += usize::from(p.iter().any(|v| !v.is_finite()));
        for v in &p[..3] {
            let linear = v.max(0.) / (1. + v.max(0.));
            let srgb = if linear <= 0.0031308 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1. / 2.4) - 0.055
            };
            rgb.push((srgb.clamp(0., 1.) * 255.).round() as u8);
        }
    }
    let mut png = png::Encoder::new(
        fs::File::create(prefix.with_extension("png"))?,
        info.width,
        info.height,
    );
    png.set_color(png::ColorType::Rgb);
    png.set_depth(png::BitDepth::Eight);
    png.set_source_srgb(png::SrgbRenderingIntent::RelativeColorimetric);
    png.write_header()?.write_image_data(&rgb)?;
    let mut pfm = std::io::BufWriter::new(fs::File::create(prefix.with_extension("pfm"))?);
    write!(pfm, "PF\n{} {}\n-1.0\n", info.width, info.height)?;
    for row in values.chunks_exact(info.width as usize * 4).rev() {
        for p in row.chunks_exact(4) {
            for v in &p[..3] {
                pfm.write_all(&v.to_le_bytes())?;
            }
        }
    }
    pfm.flush()?;
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let metadata = serde_json::json!({
        "width":info.width,"height":info.height,"samples":info.samples as u32,"frame":info.frame.0,"revision":info.revision.0,
        "format":"RGBA16F_le_tightly_packed_top_down","raw":prefix.with_extension("rgba16f"),"pfm":"exposed_linear_RGB_bottom_up_le_f32",
        "pre_exposure":info.pre_exposure,"radiance_reconstruction":"stored_exposed/pre_exposure; invalid where samples saturated",
        "shader_sha256":hex(&info.shader_sha256),"packed_lighting_sha256":info.lighting_sha256.map(|b|hex(&b)),
        "stored_resolved_saturated_texels_rgb":saturated,"stored_resolved_saturated_channels_rgb":saturated_channels,"stored_resolved_nonfinite_texels_rgba":nonfinite,
        "per_sample_saturation_detector":false,"hidden_sample_saturation_possible":info.samples==SampleCount::Four,
        "png":"sRGB_encoded_per_channel_Reinhard_of_exposed_resolved_HDR","identities":identities,
    });
    fs::write(
        prefix.with_extension("json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    eprintln!(
        "hdr_capture prefix={} pre_exposure={:?} stored_resolved_saturated_texels={saturated} nonfinite_texels={nonfinite} per_sample_saturation_detector=false",
        prefix.display(),
        info.pre_exposure
    );
    Ok(())
}
