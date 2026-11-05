"""Stage one-variable shader screens against the immutable 2268105 policy.

Diagnostic variants may change visibility and are never production candidates.
Only split-loop is general-purpose; constant-epsilon is a quarry-only cost control.
"""
import json
from pathlib import Path

OUT = Path("E:/renderlab-work/receiver-adoption/followup")
BASE = OUT / "baseline"
FILES = ("kernels-hard", "kernels-soft", "traced-hard", "traced-soft")


def edit(s, a, b):
    assert s.count(a) == 1, (a, s.count(a))
    return s.replace(a, b)


def split_loop(s):
    if "fn fresh_estimate(" not in s:
        return s
    start = s.index("fn fresh_estimate(")
    end = s.index("// Receiver pixels per workgroup", start)
    original = s[start:end]
    opaque = original.replace("fn fresh_estimate(", "fn fresh_estimate_opaque(")
    opaque = edit(opaque, "        if receiver.thin {\n            ray_origin=shadow_origin(receiver,direction);\n            ray_projected=(view.world_to_clip*vec4<f32>(ray_origin,1.0)).xyz;\n        }\n", "")
    opaque = edit(opaque, "    let origin=shadow_origin(receiver,l);", "    let viewed=Receiver(receiver.world,receiver.geometric,receiver.pixel,receiver.slot,receiver.epsilon,false);\n    let origin=shadow_origin(viewed,l);")
    thin = original.replace("fn fresh_estimate(", "fn fresh_estimate_thin(")
    wrapper = """fn fresh_estimate(receiver:Receiver,pixel:vec2<u32>,width:u32,lane:u32)->u32 {
    if receiver.thin {return fresh_estimate_thin(receiver,pixel,width,lane);}
    return fresh_estimate_opaque(receiver,pixel,width,lane);
}
"""
    return s[:start] + thin + opaque + wrapper + s[end:]


def variant(name, transform, identity=False):
    directory = OUT / name
    directory.mkdir(exist_ok=True)
    for f in FILES:
        (directory / f"{f}.wgsl").write_text(transform((BASE / f"{f}.wgsl").read_text()))
    manifest = json.loads(Path("E:/renderlab-work/receiver-adoption/static.json").read_text())
    manifest.update(output=str(OUT / (name + "-run")), frozen_modules=str(BASE),
                    candidate_modules=str(directory), warmup_pairs=12, pairs=24,
                    require_identity=identity)
    (OUT / f"{name}.json").write_text(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    variant("diag-floor", lambda s: edit(s, "receiver_epsilon(a,b,c),(primary_placements", "1e-5,(primary_placements"), True)
    variant("diag-origin", lambda s: edit(s, "return fma(normal,vec3<f32>(receiver.epsilon),receiver.world);", "return fma(normal,vec3<f32>(0.001),receiver.world);"))
    variant("diag-tmin", lambda s: s.replace("let tmin=receiver.epsilon;", "let tmin=view.coverage_bias.z/rate;"))
    variant("diag-no-traversal", lambda s: s.replace("while rayQueryProceed(&query) {}", ""))
    variant("split-loop", split_loop, True)
    for lanes in (16, 32):
        def wider(s, lanes=lanes):
            if "fn fresh_compute(" not in s:
                return s
            s = edit(s, "const FRESH_LANES:u32=8u;", f"const FRESH_LANES:u32={lanes}u;")
            return edit(s, "@compute @workgroup_size(32) fn fresh_compute", f"@compute @workgroup_size({lanes*4}) fn fresh_compute")
        variant(f"wide-{lanes}", wider, True)
    def interleaved(s):
        if "fn fresh_compute(" not in s:
            return s
        s = edit(s, "let slot=index/lanes;", "let slots=32u/lanes;let slot=index%slots;")
        s = edit(s, "id,size.x,index%lanes)", "id,size.x,index/slots)")
        return edit(s, "if inside && index%lanes==0u", "if inside && index/slots==0u")
    variant("interleaved", interleaved, True)
    manifest = json.loads((OUT / "wide-16.json").read_text())
    manifest.update(output=str(OUT / "fresh-screen-extended"), warmup_pairs=8, pairs=16, variants=[])
    for lanes, tile in [(8,(2,2)), (8,(4,1)), (8,(1,4)), (4,(4,2)), (4,(2,4)), (2,(4,4)), (1,(8,4)), (1,(4,8))]:
        name=f"fresh-{lanes}-{tile[0]}x{tile[1]}"
        source=(BASE / "kernels-soft.wgsl").read_text()
        source=edit(source,"const FRESH_LANES:u32=8u;",f"const FRESH_LANES:u32={lanes}u;")
        source=edit(source,"const FRESH_TILE=vec2<u32>(2u,2u);",f"const FRESH_TILE=vec2<u32>({tile[0]}u,{tile[1]}u);")
        path=OUT/f"{name}.wgsl";path.write_text(source)
        manifest["variants"].append(dict(name=name,source=str(path),tile=tile))
    for factor in (2,4):
        source=(BASE/"kernels-soft.wgsl").read_text()
        loop="for(var j=lane;j<FRESH_DIRECTIONS;j+=select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u)) {"
        start=source.index(loop);body_start=start+len(loop)
        depth=1;end=body_start
        while depth:
            depth += (source[end]=='{')-(source[end]=='}');end+=1
        body=source[body_start:end-1]
        step="select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u)"
        expanded=f"for(var base=lane;base<FRESH_DIRECTIONS;base+={factor}u*{step}) {{"
        for k in range(factor):expanded+=f"\n{{let j=base+{k}u*{step};if j<FRESH_DIRECTIONS {{{body}\n}}}}"
        source=source[:start]+expanded+"\n}"+source[end:]
        path=OUT/f"fresh-unroll-{factor}.wgsl";path.write_text(source)
        manifest["variants"].append(dict(name=f"fresh-unroll-{factor}",source=str(path),tile=[2,2]))
    (OUT/"fresh-screen.json").write_text(json.dumps(manifest,indent=2))
    eight_manifest=dict(manifest, output=str(OUT/"fresh-eight-screen"), variants=[])
    for lanes,tile in [(8,(2,2)),(4,(2,4)),(1,(8,4))]:
        s=(BASE/"kernels-soft.wgsl").read_text()
        for a,b in [
            ("const FRESH_LANES:u32=8u;",f"const FRESH_LANES:u32={lanes}u;"),
            ("override FRESH_DIRECTIONS:u32=256u;","const FRESH_DIRECTIONS:u32=8u;"),
            ("j+=select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u)","j+=FRESH_LANES"),
            ("let lanes=select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u);","let lanes=FRESH_LANES;"),
            ("select(FRESH_TILE,vec2<u32>(8u,4u),FRESH_DIRECTIONS==8u)","FRESH_TILE"),
            ("const FRESH_TILE=vec2<u32>(2u,2u);",f"const FRESH_TILE=vec2<u32>({tile[0]}u,{tile[1]}u);")
        ]:s=edit(s,a,b)
        name=f"fresh-eight-{lanes}";path=OUT/f"{name}.wgsl";path.write_text(s)
        eight_manifest["variants"].append(dict(name=name,source=str(path),tile=tile,directions=8))
        paired=json.loads((OUT/"wide-16.json").read_text());paired.pop("candidate_modules")
        paired.update(output=str(OUT/f"separate-eight-{lanes}-run"),separate_interactive=str(path),separate_tile=tile)
        (OUT/f"separate-eight-{lanes}.json").write_text(json.dumps(paired,indent=2))
    (OUT/"fresh-eight-screen.json").write_text(json.dumps(eight_manifest,indent=2))
    def parallel_interactive(s):
        if "fn primary_trace_interactive(" not in s:
            return s
        s=edit(s, "fn fresh_estimate(receiver:Receiver, pixel:vec2<u32>, width:u32, lane:u32) -> u32 {",
            "fn fresh_estimate_lanes(receiver:Receiver, pixel:vec2<u32>, width:u32, lane:u32, stride:u32) -> u32 {")
        s=edit(s,"j+=select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u)","j+=stride")
        start=s.index("// Interactive soft frames:")
        entry=(Path(__file__).resolve().parents[1]/"support/receiver_interactive_screen.wgsl").read_text()
        wrapper="""fn fresh_estimate(receiver:Receiver,pixel:vec2<u32>,width:u32,lane:u32)->u32 {
    return fresh_estimate_lanes(receiver,pixel,width,lane,select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u));
}
"""
        return s[:start]+wrapper+entry
    variant("interactive-parallel",parallel_interactive,True)
    def stride_helper(s):
        if "fn fresh_estimate(" not in s:return s
        s=edit(s,"fn fresh_estimate(receiver:Receiver, pixel:vec2<u32>, width:u32, lane:u32) -> u32 {",
            "fn fresh_estimate_lanes(receiver:Receiver, pixel:vec2<u32>, width:u32, lane:u32, stride:u32) -> u32 {")
        s=edit(s,"j+=select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u)","j+=stride")
        return s+"""\nfn fresh_estimate(receiver:Receiver,pixel:vec2<u32>,width:u32,lane:u32)->u32 {
    return fresh_estimate_lanes(receiver,pixel,width,lane,select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u));
}
"""
    variant("stride-helper",stride_helper,True)
    def query_helper(s):
        if "fn fresh_estimate(" not in s:return s
        s=edit(s,"    var query:ray_query;\n","")
        s=edit(s,"                let flags=RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT;\n                rayQueryInitialize(&query,fresh_scene,RayDesc(flags,255u,tmin,tmax,ray_origin,direction));\n                while rayQueryProceed(&query) {}\n                blocked=rayQueryGetCommittedIntersection(&query).kind!=0u;",
            "                blocked=fresh_occluded(tmin,tmax,ray_origin,direction);")
        return s+"""\nfn fresh_occluded(tmin:f32,tmax:f32,origin:vec3<f32>,direction:vec3<f32>)->bool {
    var query:ray_query;
    rayQueryInitialize(&query,fresh_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT,255u,tmin,tmax,origin,direction));
    while rayQueryProceed(&query) {}
    return rayQueryGetCommittedIntersection(&query).kind!=0u;
}
"""
    variant("query-helper",query_helper,True)
    def first_candidate(s):
        if "fn fresh_estimate(" not in s:return s
        s=edit(s,"let flags=RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT;","let flags=RAY_FLAG_FORCE_NO_OPAQUE;")
        return edit(s,"                while rayQueryProceed(&query) {}\n                blocked=rayQueryGetCommittedIntersection(&query).kind!=0u;",
            "                blocked=rayQueryProceed(&query);\n                if blocked {rayQueryTerminate(&query);}")
    variant("first-candidate",first_candidate,True)
    variant("closest-query",lambda s:s.replace("let flags=RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT;","let flags=RAY_FLAG_FORCE_OPAQUE;"),True)
    manifest=json.loads((OUT/"wide-16.json").read_text())
    manifest.update(output=str(OUT/"hard-shade-screen"),hard=True,variants=[dict(name="shade-8x4",stage="shade",tile=[8,4],edits=[
        ["@compute @workgroup_size(4,4) fn primary_shade_tiles","@compute @workgroup_size(8,4) fn primary_shade_tiles"],
        ["let shaded=(region.zw-region.xy)/vec2<u32>(4u);","let shaded=(region.zw-region.xy)/vec2<u32>(8u,4u);"],
        ["if group.y>=shaded.y {","if group.y>=shaded.y {\n        if lane>=16u {return;}"],
    ])],warmup_reps=16,reps=32)
    (OUT/"hard-shade-screen.json").write_text(json.dumps(manifest,indent=2))
