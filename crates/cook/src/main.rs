use renderlab_cook::{
    CookLimits, MissingNormals, Settings, cook_file, fixtures, hash_hex, load_manifest,
};
use serde_json::json;
use std::{env, path::Path, process::ExitCode};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["fixtures",directory]=> {
            fixtures::generate(Path::new(directory))?;
            for (name,bytes) in fixtures::sources()? { println!("{}",json!({"source":name,"bytes":bytes.len(),"sha256":hash_hex(&bytes)})); }
        },
        ["cook",source,cache,manifest] | ["cook",source,cache,manifest,"--reject-missing-normals"]=> {
            let settings=Settings { missing_normals:if args.len()==5 { MissingNormals::Reject } else { MissingNormals::Flat } };
            let result=cook_file(Path::new(source),Path::new(cache),Path::new(manifest),settings,CookLimits::default())?;
            let r=result.report;let c=r.counts;
            println!("{}",json!({"source":source,"manifest":manifest,"key":result.loaded.key,"payload_sha256":result.loaded.payload_hash,
                "geometry_sha256":result.loaded.geometry_hashes,"cache_hit":r.cache_hit,"cache_miss_reason":r.cache_miss_reason,
                "source_bytes":r.source_bytes,"prepared_bytes":r.prepared_bytes,"geometry_sidecar_bytes":r.geometry_sidecar_bytes,
                "reused_geometry_entries":r.reused_geometry_entries,"reused_geometry_bytes":r.reused_geometry_bytes,
                "counts":{"meshes":c.meshes,"primitives":c.primitives,"placements":c.placements,"materials":c.materials,"textures":c.textures,
                    "vertices":c.vertices,"indices":c.indices,"mip_levels":c.mip_levels,"texture_bytes":c.texture_bytes,"decoded_bytes":c.decoded_bytes},
                "timings_us":{"source_read":r.source_read_us,"lookup_and_verify":r.lookup_us,"prepare":r.prepare_us,"encode_and_hash":r.encode_us,
                    "decode_and_geometry_verify":r.load_us,"publication":r.publication_us,"total":r.total_us}}));
        },
        ["inspect",cache,manifest]=> {
            let loaded=load_manifest(Path::new(cache),Path::new(manifest),CookLimits::default())?;
            let c=loaded.asset.validate(CookLimits::default().prepared)?;
            println!("{}",json!({"key":loaded.key,"payload_sha256":loaded.payload_hash,"meshes":c.meshes,"primitives":c.primitives,"placements":c.placements,
                "vertices":c.vertices,"indices":c.indices,"textures":c.textures,"mip_levels":c.mip_levels,"decoded_bytes":c.decoded_bytes}));
        },
        _=>return Err("usage: renderlab-cook fixtures DIRECTORY | cook SOURCE.glb CACHE CURRENT.manifest [--reject-missing-normals] | inspect CACHE MANIFEST".into())
    }
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("renderlab-cook: {e}");
            ExitCode::FAILURE
        }
    }
}
