// Group sphere (centre, radius) and accumulated simplification error; terminal groups
// carry f32::MAX and never coarsen.
struct Group {sphere:vec4<f32>,error:f32,pad0:u32,pad1:u32,pad2:u32};
@group(0) @binding(2) var<storage,read> groups:array<Group>;
// Software clusters from 0, hardware clusters from the cluster count.
@group(0) @binding(3) var<storage,read_write> lists:array<u32>;
// Software dispatch (x, y, z) and its cluster count, the hardware draw, statistics (cut
// clusters and triangles in pass 1, software and hardware triangles), then pass 1's
// dispatch of pass 2's cull: a buffer pass 2 does not bind, so it can be indirect.
struct Args {sw_x:u32,sw_y:u32,sw_z:u32,sw_count:atomic<u32>,hw_vertices:u32,hw_instances:atomic<u32>,hw_first:u32,hw_first_instance:u32,
    cut_clusters:atomic<u32>,cut_triangles:atomic<u32>,sw_triangles:atomic<u32>,hw_triangles:atomic<u32>,next_x:u32,next_y:u32,next_z:u32,pad:u32};
@group(0) @binding(5) var<storage,read_write> args:Args;
// Pass-1 HZB rejects: their count, each pass's pyramid dispatches (16 levels of x, y,
// z; zero skips a pyramid nothing reads), then the reject indices.
struct Rejects {pad0:u32,pad1:u32,pad2:u32,count:atomic<u32>,pyramid:array<u32,96>,items:array<u32>};
@group(0) @binding(6) var<storage,read_write> rejects:Rejects;
@group(0) @binding(7) var hzb:texture_2d<f32>;

// Projected simplification error in pixels: meshoptimizer clusterlod's rotationally
// invariant estimate, with a conservative affine scale (sqrt of 1-norm * inf-norm).
fn group_error(g:u32,placement:u32)->f32 {
    let s=groups[g];
    if s.error>=3.0e38 {return 3.0e38;}
    let m=models[placement];
    let a=abs(m[0].xyz);let b=abs(m[1].xyz);let c=abs(m[2].xyz);
    let columns=max(a.x+a.y+a.z,max(b.x+b.y+b.z,c.x+c.y+c.z));
    let rows=max(a.x+b.x+c.x,max(a.y+b.y+c.y,a.z+b.z+c.z));
    let scale=sqrt(columns*rows);
    let centre=(m*vec4<f32>(s.sphere.xyz,1.0)).xyz;
    let distance=max(length(centre-config.eye.xyz)-s.sphere.w*scale,config.lod.y);
    return s.error*scale/distance*config.eye.w;
}
// clusterlod's cut: the owner group is too coarse to stop at, and the group this
// cluster was simplified from (if any) is fine enough to replace.
fn in_cut(c:Cluster)->bool {
    let tau=config.lod.x;
    return group_error(c.owner,c.placement)>tau && (c.refined==0xffffffffu || group_error(c.refined,c.placement)<=tau);
}
fn occluded(lo:vec4<f32>,hi:vec4<f32>)->bool {
    if hi.z>=1.0 {return false;}
    let size=vec2<f32>(image_size());
    let a=max(vec2<f32>(0.0),floor(vec2<f32>(lo.x+1.0,1.0-hi.y)*0.5*size)-vec2<f32>(1.0));
    let b=min(size-vec2<f32>(1.0),ceil(vec2<f32>(hi.x+1.0,1.0-lo.y)*0.5*size)+vec2<f32>(1.0));
    // Level k texels span 2^(k+1) pixels, so the rectangle covers at most 2x2 texels.
    let span=u32(ceil(log2(max(max(b.x-a.x+1.0,b.y-a.y+1.0),1.0))));
    let level=min(max(span,1u)-1u,textureNumLevels(hzb)-1u);
    let p=vec2<i32>(a)>>vec2<u32>(level+1u);let q=vec2<i32>(b)>>vec2<u32>(level+1u);
    let depth=min(min(textureLoad(hzb,p,i32(level)).x,textureLoad(hzb,q,i32(level)).x),min(textureLoad(hzb,vec2<i32>(p.x,q.y),i32(level)).x,textureLoad(hzb,vec2<i32>(q.x,p.y),i32(level)).x));
    return hi.z<depth;
}
// Low bits 0: not drawn, 1: software, 2: hardware; 4 marks a pass-1 cut cluster and 8 a
// pass-1 reject. Pass 1 tests the cut against the previous frame's HZB with the previous
// camera; pass 2 retests only those rejects against this frame's pass-1 HZB. Final
// visibility never depends on the previous frame.
fn classify(index:u32,second:bool)->u32 {
    let c=clusters[index];
    if !second && !in_cut(c) {return 0u;}
    let cut=select(0u,4u,!second);
    let b=cluster_box(c,config.matrix);let lo=b[0];let hi=b[1];
    if lo.x>1.0 || hi.x< -1.0 || lo.y>1.0 || hi.y< -1.0 || hi.z<0.0 {return cut;}
    if second {
        if occluded(lo,hi) {return 0u;}
    } else if config.info.y!=0u {
        let p=cluster_box(c,config.previous);
        if occluded(p[0],p[1]) {return cut|8u;}
    }
    let extent=(hi.xy-lo.xy)*0.5*vec2<f32>(image_size());
    if hi.z>=1.0 || max(extent.x,extent.y)>config.lod.z {return cut|2u;}
    return cut|1u;
}
var<workgroup> wg_counts:array<atomic<u32>,7>;
var<workgroup> wg_bases:array<u32,3>;
@compute @workgroup_size(64) fn cull(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let thread=id.x+id.y*65535u*64u;
    let second=config.info.z!=0u;
    var index=thread;
    var valid=thread<config.info.x;
    if second {
        valid=thread<atomicLoad(&rejects.count);
        if valid {index=rejects.items[thread];}
    }
    var kind=0u;var triangles=0u;
    if valid {
        kind=classify(index,second);
        triangles=clusters[index].counts&255u;
    }
    var slot=0u;var reject=0u;
    if (kind&3u)!=0u {slot=atomicAdd(&wg_counts[(kind&3u)-1u],1u);atomicAdd(&wg_counts[(kind&3u)+3u],triangles);}
    if (kind&4u)!=0u {atomicAdd(&wg_counts[2],1u);atomicAdd(&wg_counts[3],triangles);}
    if (kind&8u)!=0u {reject=atomicAdd(&wg_counts[6],1u);}
    workgroupBarrier();
    if li==0u {
        wg_bases[0]=atomicAdd(&args.sw_count,atomicLoad(&wg_counts[0]));
        wg_bases[1]=atomicAdd(&args.hw_instances,atomicLoad(&wg_counts[1]));
        wg_bases[2]=atomicAdd(&rejects.count,atomicLoad(&wg_counts[6]));
        atomicAdd(&args.cut_clusters,atomicLoad(&wg_counts[2]));
        atomicAdd(&args.cut_triangles,atomicLoad(&wg_counts[3]));
        atomicAdd(&args.sw_triangles,atomicLoad(&wg_counts[4]));
        atomicAdd(&args.hw_triangles,atomicLoad(&wg_counts[5]));
    }
    workgroupBarrier();
    if (kind&3u)==1u {lists[wg_bases[0]+slot]=index;}
    if (kind&3u)==2u {lists[config.info.x+wg_bases[1]+slot]=index;}
    if (kind&8u)!=0u {rejects.items[wg_bases[2]+reject]=index;}
}
// One invocation after each pass's cull: indirect raster arguments, pass 2's cull
// dispatch, and that pass's pyramid. Pass 1's pyramid is read only by pass 2's retest
// of its rejects. The final pyramid (for the next frame) is redundant when pass 2 drew
// nothing over pass 1's built pyramid.
@compute @workgroup_size(1) fn finish() {
    let n=atomicLoad(&args.sw_count);
    args.sw_x=min(n,65535u);args.sw_y=(n+65534u)/65535u;args.sw_z=1u;
    args.hw_vertices=384u;args.hw_first=0u;args.hw_first_instance=0u;
    let r=atomicLoad(&rejects.count);
    let second=config.info.z!=0u;
    if !second {
        let groups=(r+63u)/64u;
        args.next_x=min(groups,65535u);args.next_y=(groups+65534u)/65535u;args.next_z=1u;
    }
    var build=r!=0u;
    if second {build=r==0u || n+atomicLoad(&args.hw_instances)!=0u;}
    let levels=min(textureNumLevels(hzb),16u);
    for(var l=0u;l<levels;l+=1u) {
        let d=(textureDimensions(hzb,l)+vec2<u32>(7u))/8u;
        let o=config.info.z*48u+l*3u;
        rejects.pyramid[o]=select(0u,d.x,build);rejects.pyramid[o+1u]=select(0u,d.y,build);rejects.pyramid[o+2u]=1u;
    }
}
