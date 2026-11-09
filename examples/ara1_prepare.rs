//! CPU-only GLB -> shared RLA preparation for the Ara/Renderlab measurement harness.
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() == 3 && args[0] == "--orient" {
        let mut asset =
            renderlab::asset::decode(&fs::read(&args[1])?, renderlab::asset::Limits::unlimited())?;
        let mut flipped = 0usize;
        let mut kept = 0usize;
        for mesh in &mut asset.meshes {
            for p in &mut mesh.primitives {
                for triangle in p.indices.chunks_exact_mut(3) {
                    let [a, b, c] =
                        [triangle[0], triangle[1], triangle[2]].map(|i| &p.vertices[i as usize]);
                    let position =
                        |v: &renderlab::asset::Vertex| renderlab::Vec3::from_array(v.position);
                    let normal = renderlab::Vec3::from_array(a.normal)
                        + renderlab::Vec3::from_array(b.normal)
                        + renderlab::Vec3::from_array(c.normal);
                    if (position(b) - position(a))
                        .cross(position(c) - position(a))
                        .dot(normal)
                        < 0.0
                    {
                        triangle.swap(1, 2);
                        flipped += 1;
                    } else {
                        kept += 1;
                    }
                }
            }
        }
        let out = PathBuf::from(&args[2]);
        fs::write(
            &out,
            renderlab::asset::encode(&asset, renderlab::asset::Limits::unlimited())?,
        )?;
        fs::write(
            out.with_extension("orientation.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "source":PathBuf::from(&args[1]),"flipped_triangles":flipped,"kept_triangles":kept,
                "method":"swap indices 1/2 where geometric normal opposes summed authored vertex normals; positions, normals, tangents, UV, material and texture bytes unchanged; same canonical result for both engines"
            }))?,
        )?;
        println!("flipped={flipped} kept={kept}");
        return Ok(());
    }
    if args.len() != 2 {
        return Err("ara1_prepare SOURCE.glb OUTPUT.rla".into());
    }
    let source = fs::read(&args[0])?;
    if args[1] == "--inspect" {
        let asset = renderlab::asset::decode(&source, renderlab::asset::Limits::unlimited())?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "materials":asset.materials.iter().map(|m|format!("{m:?}")).collect::<Vec<_>>(),
                "textures":asset.textures.iter().map(|t|serde_json::json!({"role":format!("{:?}",t.role),
                    "size":[t.mips[0].width,t.mips[0].height],"first_pixel":&t.mips[0].rgba8[..4]})).collect::<Vec<_>>(),
                "meshes":asset.meshes.iter().map(|m| m.primitives.iter().map(|p|serde_json::json!({"vertices":p.vertices.len(),
                    "triangles":p.indices.len()/3,"material":p.material,"first_vertex":format!("{:?}",p.vertices[0]),
                    "first_tangent":p.tangents.as_ref().map(|t|t.values[0])})).collect::<Vec<_>>()).collect::<Vec<_>>()
            }))?
        );
        return Ok(());
    }
    let limits = renderlab_cook::CookLimits::unlimited();
    let asset = renderlab_cook::prepare(
        &source,
        renderlab_cook::Settings {
            normal_maps: true,
            ..Default::default()
        },
        limits,
    )?;
    let output = PathBuf::from(&args[1]);
    fs::create_dir_all(output.parent().ok_or("output has no parent")?)?;
    fs::write(&output, renderlab::asset::encode(&asset, limits.prepared)?)?;
    eprintln!(
        "{}: {} meshes, {} placements",
        output.display(),
        asset.meshes.len(),
        asset.placements.len()
    );
    Ok(())
}
