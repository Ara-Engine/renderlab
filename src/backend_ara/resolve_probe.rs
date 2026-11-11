//! MEASUREMENT ONLY: a one-pixel trace of the temporal resolve.
//!
//! With `RENDERLAB_ARA_PROBE_RESOLVE=X,Y` set, the resolve's source gains a storage
//! buffer and, for that one output pixel, writes what it decided and on what: its
//! surfaces, the path it took, each fetch position, the four coverage taps with the
//! ids and coverage they read, and what it stored. The frame reads the buffer back
//! ([`ForwardImages::resolve_probe`](super::forward::ForwardImages)) and `ara_frame`
//! prints it with [`describe`]. Run on two devices, the first record where they part
//! names the decision that differs. Unset, the resolve's source is unchanged.
//!
//! Records are written in program order, two `vec4<f32>` each: a header (tag, the tap
//! or call index, 0, 0) and the value. Ids are written as `f32` (exact below 2^24;
//! `TR1_NONE` shows as 4294967296).
use anyhow::{Result, ensure};
use std::sync::OnceLock;

/// The probe buffer's binding in group 0; the resolve uses 40 to 48.
pub(crate) const BINDING: u32 = 120;
/// Records the buffer holds; later ones are dropped.
pub(crate) const RECORDS: usize = 64;
/// The buffer's size in bytes: a count word's `vec4`, then the records.
pub(crate) const BYTES: u64 = (16 + RECORDS * 32) as u64;

/// The pixel `RENDERLAB_ARA_PROBE_RESOLVE` names, read once.
pub(crate) fn pixel() -> Option<[u32; 2]> {
    static PIXEL: OnceLock<Option<[u32; 2]>> = OnceLock::new();
    *PIXEL.get_or_init(|| {
        let value = std::env::var("RENDERLAB_ARA_PROBE_RESOLVE").ok()?;
        let (x, y) = value.split_once(',')?;
        Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
    })
}

/// Each seam in the resolve's source, the text inserted after it, and a name for the
/// record's tag. Every seam must occur exactly once.
const AFTER: [(&str, &str); 4] = [
    (
        "    tr1_here=vec2<i32>(id.xy);\n",
        "    tr1_probing=all(id.xy==TR1_PROBE_PIXEL);\n",
    ),
    (
        "    let s=tr1_surface(p,size);\n",
        "    tr1_put(1u,0u,vec4<f32>(f32(s.id),s.distance,select(0.0,1.0,s.moved),f32(s.side)));\n",
    ),
    (
        "    let sample=textureLoad(tr1_sample,tr1_map(p),0).xyz;\n",
        "    tr1_put(2u,0u,vec4<f32>(sample,f32(other)));\n",
    ),
    (
        "    let f=q-floor(q);\n    var total=0.0;var coverage=0.0;var weight=0.0;\n",
        "    tr1_put(20u,0u,vec4<f32>(q,f));\n",
    ),
];
/// Seams the records go before.
const BEFORE: [(&str, &str); 9] = [
    (
        "                textureStore(tr1_output,p,vec4<f32>(mix(theirs,mine.xyz,own.coverage),1.0));\n",
        "                tr1_put(3u,0u,vec4<f32>(mix(theirs,mine.xyz,own.coverage),own.coverage));\n",
    ),
    (
        "        tr1_store_single(p,s,tr1_continue(p,s,sample,size));\n",
        "        tr1_put(4u,0u,tr1_continue(p,s,sample,size));\n",
    ),
    (
        "    var o=u;\n",
        "    tr1_put(5u,0u,vec4<f32>(f32(u.id),u.distance,select(0.0,1.0,occluder_is_s),f32(kept.id)));\n",
    ),
    (
        "    var coverage=tr1_share(o.id);\n",
        "    tr1_put(6u,0u,mine);tr1_put(7u,0u,theirs);tr1_put(8u,0u,vec4<f32>(c,f32(o.id)));\n",
    ),
    (
        "    let film=select(select(mix(back,front,coverage),back,coverage<=0.0),front,coverage>=1.0);\n",
        "    tr1_put(9u,0u,vec4<f32>(coverage,coverage_weight,tr1_share(o.id),0.0));\n",
    ),
    (
        "    textureStore(tr1_output,p,vec4<f32>(film,1.0));\n}\n",
        "    tr1_put(10u,0u,vec4<f32>(film,0.0));\n",
    ),
    (
        "    let r=round(q);\n",
        "    tr1_put(21u,0u,vec4<f32>(q,f32(s.id),0.0));\n",
    ),
    (
        "        if c>=0.0 && n>0.0 {\n            let w=select(1.0-f.x,f.x,d.x!=0)*select(1.0-f.y,f.y,d.y!=0);\n",
        "        tr1_put(22u,u32(i),vec4<f32>(f32(first),f32(second.id),c,n));\n        tr1_put(23u,u32(i),vec4<f32>(f32(t.x),f32(t.y),second.coverage,second.coverage_weight));\n",
    ),
    (
        "    if total<=0.0 {return vec3<f32>(0.0);}\n",
        "    tr1_put(24u,0u,vec4<f32>(total,coverage,weight,f32(u)));\n",
    ),
];

/// The resolve's `source` with the probe written in for `pixel`.
pub(crate) fn instrument(source: &str, [x, y]: [u32; 2]) -> Result<String> {
    let mut out = source.to_owned();
    for (seam, record) in AFTER {
        ensure!(
            out.matches(seam).count() == 1,
            "resolve probe seam changed: {seam:?}"
        );
        out = out.replacen(seam, &format!("{seam}{record}"), 1);
    }
    for (seam, record) in BEFORE {
        ensure!(
            out.matches(seam).count() == 1,
            "resolve probe seam changed: {seam:?}"
        );
        out = out.replacen(seam, &format!("{record}{seam}"), 1);
    }
    let entry = "@compute @workgroup_size(8,8) fn tr1_resolve(";
    ensure!(
        out.matches(entry).count() == 1,
        "resolve probe entry changed"
    );
    let api = format!(
        "@group(0) @binding({BINDING}) var<storage,read_write> tr1_probe:array<vec4<f32>,{}>;\n\
         const TR1_PROBE_PIXEL:vec2<u32>=vec2<u32>({x}u,{y}u);\n\
         var<private> tr1_probing:bool;\n\
         var<private> tr1_probe_count:u32;\n\
         fn tr1_put(tag:u32,index:u32,value:vec4<f32>) {{\n\
         \x20   if !tr1_probing || tr1_probe_count>={RECORDS}u {{return;}}\n\
         \x20   tr1_probe[1u+2u*tr1_probe_count]=vec4<f32>(f32(tag),f32(index),0.0,0.0);\n\
         \x20   tr1_probe[2u+2u*tr1_probe_count]=value;\n\
         \x20   tr1_probe_count+=1u;\n\
         \x20   tr1_probe[0]=vec4<f32>(f32(tr1_probe_count),0.0,0.0,0.0);\n\
         }}\n",
        1 + 2 * RECORDS
    );
    Ok(out.replacen(entry, &format!("{api}{entry}"), 1))
}

/// What a record's tag names, for [`describe`].
fn name(tag: u32) -> &'static str {
    match tag {
        1 => "s: id, distance, moved, side",
        2 => "sample rgb, other ring index",
        3 => "third surface: film rgb, coverage",
        4 => "one surface: continued rgb, weight",
        5 => "u: id, distance, occluder is s, kept id",
        6 => "mine: rgb, weight",
        7 => "theirs: rgb, weight",
        8 => "coverage continued: c, weight, window, occluder id",
        9 => "stored: coverage, coverage weight, share",
        10 => "film rgb",
        20 => "coverage fetch: q, f",
        21 => "fetch position: q, surface id",
        22 => "tap: first id, second id, c, n",
        23 => "tap: x, y, stored coverage, coverage weight",
        24 => "taps: total, coverage, weight, u",
        _ => "?",
    }
}

/// The read-back buffer as one line a record, in the order the resolve wrote them.
pub fn describe(words: &[[f32; 4]]) -> Vec<String> {
    let count = words.first().map_or(0, |w| w[0] as usize).min(RECORDS);
    (0..count)
        .filter_map(|k| {
            let [tag, index, ..] = *words.get(1 + 2 * k)?;
            let value = words.get(2 + 2 * k)?;
            Some(format!(
                "{:>2} {} [{}]: {:?}",
                tag as u32,
                name(tag as u32),
                index as u32,
                value
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    /// Every seam is in the shared resolve, and the traced source still reflects with
    /// the probe buffer at its binding.
    #[test]
    fn the_probe_finds_its_seams_and_reflects() {
        let source = crate::renderer::primary_ray_gpu::temporal_reconstruction::resolve_source(
            include_str!("../../shaders/scene.wgsl"),
        )
        .expect("resolve source");
        let traced = super::instrument(&source, [3, 4]).expect("seams");
        let program =
            super::super::shader::Program::new(super::super::shader::Pass::TemporalResolve, traced)
                .expect("traced resolve reflects");
        assert!(
            program
                .inputs()
                .iter()
                .any(|i| (i.key.group, i.key.binding) == (0, super::BINDING))
        );
    }
}
