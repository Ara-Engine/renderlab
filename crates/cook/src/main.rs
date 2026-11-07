use renderlab_asset as asset;
#[path = "../../../support/limits.rs"]
mod policy;
use renderlab_cook::{
    CookLimits, MissingNormals, Settings, cook_file, fixtures, hash_hex, load_manifest,
};
use serde_json::json;
use std::{env, path::Path, process::ExitCode};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let (policy, args) = policy::Policy::args(env::args().skip(1).collect())?;
    let mut limits = CookLimits {
        prepared: policy.prepared(),
        ..Default::default()
    };
    limits.source_bytes = policy.size("cook", "source_bytes", limits.source_bytes);
    limits.json_bytes = policy.size("cook", "json_bytes", limits.json_bytes);
    limits.nodes = policy.size("cook", "nodes", limits.nodes);
    limits.hierarchy_depth = policy.size("cook", "hierarchy_depth", limits.hierarchy_depth);
    limits.images = policy.size("cook", "images", limits.images);
    limits.image_decoded_bytes =
        policy.size("cook", "image_decoded_bytes", limits.image_decoded_bytes);
    limits.accessors = policy.size("cook", "accessors", limits.accessors);
    limits.buffer_views = policy.size("cook", "buffer_views", limits.buffer_views);
    limits.scenes = policy.size("cook", "scenes", limits.scenes);
    limits.samplers = policy.size("cook", "samplers", limits.samplers);
    limits.json_values = policy.size("cook", "json_values", limits.json_values);
    limits.manifest_bytes = policy.size("cook", "manifest_bytes", limits.manifest_bytes);
    let virtual_limits = policy.virtual_limits();
    let effective = json!({"requested":policy.requested(), "cook":{
        "source_bytes":policy::cap(limits.source_bytes),
        "json_bytes":policy::cap(limits.json_bytes),
        "nodes":policy::cap(limits.nodes),
        "hierarchy_depth":policy::cap(limits.hierarchy_depth),
        "images":policy::cap(limits.images),
        "image_decoded_bytes":policy::cap(limits.image_decoded_bytes),
        "accessors":policy::cap(limits.accessors),
        "buffer_views":policy::cap(limits.buffer_views),
        "scenes":policy::cap(limits.scenes),
        "samplers":policy::cap(limits.samplers),
        "json_values":policy::cap(limits.json_values),
        "manifest_bytes":policy::cap(limits.manifest_bytes),
    },"prepared":policy::prepared_json(limits.prepared),"virtual":policy::virtual_json(virtual_limits)});
    eprintln!("effective_limits {effective}");
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["m7-bvh",source_cache,source_manifest,cache,manifest,errors,flags @ ..] if flags.iter().all(|f|*f=="--validate") => {
            use renderlab_cook::{cluster_bvh,cluster_bvh_cache,cluster_bvh_validation,virtual_cache};
            use renderlab_asset::cluster_bvh as format;
            let start = std::time::Instant::now();
            let source_cache = Path::new(source_cache);
            let source = cluster_bvh_cache::load_geometry(source_cache,Path::new(source_manifest),virtual_limits)?;
            let source_us = start.elapsed().as_micros();
            let settings = cluster_bvh::Settings{errors:errors.split(',').map(str::parse).collect::<Result<Vec<f32>,_>>()?,limits:format::Limits::default()};
            let cooked = cluster_bvh_cache::cook(&source,source_cache,Path::new(cache),Path::new(manifest),&settings,virtual_limits)?;
            let b = &cooked.bvh;
            let mut deterministic = None;
            let mut validation = None;
            let verify = std::time::Instant::now();
            if flags.contains(&"--validate") {
                let again = cluster_bvh::prepare(&source.package,&settings,virtual_limits,|p|virtual_cache::load_page(source_cache,&source.package.pages[p]))?;
                deterministic = Some(renderlab_cook::hash_hex(&format::encode(&again,settings.limits)?)==cooked.digest.iter().map(|v|format!("{v:02x}")).collect::<String>());
                if deterministic != Some(true) {return Err("BVH rebuild is not byte-identical".into());}
                drop(again);
                validation = Some(cluster_bvh_validation::validate(b,&source.package,virtual_limits,|p|virtual_cache::load_page(source_cache,&source.package.pages[p]),10_000,8)?);
            }
            let levels:Vec<_> = b.levels.iter().map(|l|json!({"mesh":l.mesh,"requested_error":l.requested_error,"max_error":l.max_error,"clusters":l.leaves.count,"triangles":b.level_leaves[l.leaves.range(b.level_leaves.len()).unwrap()].iter().map(|&id|u64::from(b.leaves[id as usize].indices.count/3)).sum::<u64>()})).collect();
            println!("{}",json!({"algorithm":String::from_utf8_lossy(cluster_bvh::ALGORITHM),"source_digest":source.digest.iter().map(|v|format!("{v:02x}")).collect::<String>(),"source_rla_version":source.rla_version,"source_rvg_version":source.rvg_version,"bvh_version":format::VERSION,"digest":cooked.digest.iter().map(|v|format!("{v:02x}")).collect::<String>(),"cache_hit":cooked.cache_hit,"gpu_bytes":b.gpu_bytes(),"encoded_bytes":b.gpu_bytes()+32,"nodes":b.nodes.len(),"leaves":b.leaves.len(),"positions":b.positions.len(),"indices":b.indices.len(),"levels":levels,"deterministic_rebuild":deterministic,"timings_us":{"load_source":source_us,"build":cooked.build_us,"cook_including_publication":cooked.total_us,"verification":verify.elapsed().as_micros(),"total":start.elapsed().as_micros()},"validation":validation}));
        },
        ["m4-register", directory] => renderlab_cook::virtual_fixtures::register(Path::new(directory))?,
        ["m4-cook", source, cache, manifest] | ["m4-cook", source, cache, manifest, "--triangles64"] => {
            use renderlab_cook::{virtual_cache,virtual_cook};
            let settings=virtual_cook::Settings {max_triangles:if args.len()==5 {64}else{128}};
            let result=virtual_cache::cook_file(Path::new(source),Path::new(cache),Path::new(manifest),settings,virtual_limits)?;
            let loaded=result.loaded;let r=result.report;let c=loaded.counts;
            let mut roots=std::collections::BTreeSet::new();let mut root_clusters=0;
            for mesh in &loaded.package.meshes {for g in mesh.terminal_groups() {for cluster in &mesh.clusters[mesh.groups[g.index()].clusters.range(mesh.clusters.len())?] {roots.insert(cluster.page);root_clusters+=1;}}}
            let stats=r.statistics.map(|s|json!({"source_vertices":s.source_vertices,"source_indices":s.source_indices,"packed_vertices":s.packed_vertices,"packed_indices":s.packed_indices,"unique_referenced_source_vertices":s.unique_referenced_source_vertices,"duplicated_vertex_records":s.packed_vertices-s.unique_referenced_source_vertices,"locked_subset_boundary_vertices":s.source_boundary_vertices_locked,"copied_callback_index_bytes":s.copied_callback_index_bytes,"peak_single_primitive_attribute_bytes":s.peak_single_primitive_attribute_bytes}));
            println!("{}",json!({"limits":effective,"key":loaded.key,"package_sha256":loaded.package_sha256,"source_sha256":hash_hex(&std::fs::read(source)?),"cache_hit":r.cache_hit,"miss_reason":r.miss_reason,"source_bytes":r.source_bytes,"metadata_file_bytes":loaded.metadata_file_bytes,"counts":{"groups":c.groups,"clusters":c.clusters,"pages":c.pages,"payload_bytes":c.payload_bytes,"disk_page_bytes":c.disk_page_bytes,"disk_padding_bytes":c.disk_page_bytes-c.payload_bytes,"metadata_host_bytes":c.metadata_host_bytes,"minimum_cover_clusters":root_clusters,"pinned_root_pages":roots.len(),"pinned_root_bytes":roots.len()*65536,"normal_pool_bytes":(8usize<<20).max((roots.len()+4)*65536)},"cooking":stats,"timings_us":{"prepare":r.prepare_us,"verify":r.verify_us,"total":r.total_us}}));
        },
        ["m4-inspect", cache, manifest] => {
            let loaded=renderlab_cook::virtual_cache::load_manifest(Path::new(cache),Path::new(manifest),virtual_limits)?;
            println!("{}",json!({"limits":effective,"key":loaded.key,"package_sha256":loaded.package_sha256,"groups":loaded.counts.groups,"clusters":loaded.counts.clusters,"pages":loaded.counts.pages,"metadata_host_bytes":loaded.counts.metadata_host_bytes}));
        },
        ["m4-prove", cache, manifest, kind] if ["closed","open"].contains(kind) => {
            let cache=Path::new(cache);let loaded=renderlab_cook::virtual_cache::load_manifest(cache,Path::new(manifest),virtual_limits)?;
            let pages=loaded.package.pages.iter().map(|p|renderlab_cook::virtual_cache::load_page(cache,p)).collect::<Result<Vec<_>,_>>()?;
            println!("{}",renderlab_cook::virtual_proof::prove(&loaded.package,&pages,*kind=="closed",false)?);
        },
        ["fixtures",directory]=> {
            fixtures::generate(Path::new(directory))?;
            for (name,bytes) in fixtures::sources()? { println!("{}",json!({"limits":effective,"source":name,"bytes":bytes.len(),"sha256":hash_hex(&bytes)})); }
        },
        ["cook",source,cache,manifest,flags @ ..] if flags.iter().all(|f| matches!(*f,"--reject-missing-normals" | "--normal-maps" | "--normal-variance")) => {
            let settings=Settings { missing_normals:if flags.contains(&"--reject-missing-normals") { MissingNormals::Reject } else { MissingNormals::Flat }, normal_maps:flags.contains(&"--normal-maps"), normal_variance:flags.contains(&"--normal-variance") };
            let result=cook_file(Path::new(source),Path::new(cache),Path::new(manifest),settings,limits)?;
            let r=result.report;let c=r.counts;
            println!("{}",json!({"limits":effective,"source":source,"manifest":manifest,"key":result.loaded.key,"payload_sha256":result.loaded.payload_hash,
                "geometry_sha256":result.loaded.geometry_hashes,"cache_hit":r.cache_hit,"cache_miss_reason":r.cache_miss_reason,
                "source_bytes":r.source_bytes,"prepared_bytes":r.prepared_bytes,"geometry_sidecar_bytes":r.geometry_sidecar_bytes,
                "reused_geometry_entries":r.reused_geometry_entries,"reused_geometry_bytes":r.reused_geometry_bytes,
                "counts":{"meshes":c.meshes,"primitives":c.primitives,"placements":c.placements,"materials":c.materials,"textures":c.textures,
                    "vertices":c.vertices,"indices":c.indices,"mip_levels":c.mip_levels,"texture_bytes":c.texture_bytes,"decoded_bytes":c.decoded_bytes},
                "timings_us":{"source_read":r.source_read_us,"lookup_and_verify":r.lookup_us,"prepare":r.prepare_us,"encode_and_hash":r.encode_us,
                    "decode_and_geometry_verify":r.load_us,"publication":r.publication_us,"total":r.total_us}}));
        },
        ["inspect",cache,manifest]=> {
            let loaded=load_manifest(Path::new(cache),Path::new(manifest),limits)?;
            let c=loaded.asset.validate(limits.prepared)?;
            println!("{}",json!({"limits":effective,"key":loaded.key,"payload_sha256":loaded.payload_hash,"meshes":c.meshes,"primitives":c.primitives,"placements":c.placements,
                "vertices":c.vertices,"indices":c.indices,"textures":c.textures,"mip_levels":c.mip_levels,"decoded_bytes":c.decoded_bytes}));
        },
        _=>return Err("usage: renderlab-cook fixtures DIRECTORY | cook SOURCE.glb CACHE CURRENT.manifest [--reject-missing-normals] [--normal-maps] [--normal-variance] | inspect CACHE MANIFEST".into())
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
