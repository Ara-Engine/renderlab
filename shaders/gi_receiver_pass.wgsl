// GI receiver output: 48 bytes/pixel, two f32 scales and 39 snorm8 coefficients.
// Three vector words retain the original byte layout, with vector loads/stores.
struct GiPrepared { r0:vec4<u32>, r1:vec4<u32>, r2:vec4<u32> };
@group(0) @binding(68) var<storage,read_write> gi_prepared:array<GiPrepared>;
fn gi_pass_load(input:SurfaceInput,id:vec2<u32>,size:vec2<u32>) {
    let at=id.y*size.x+id.x;
    let a=gi_prepared[at].r0;let b=gi_prepared[at].r1;let c=gi_prepared[at].r2;
    let scales=bitcast<vec2<f32>>(a.xy);
    gi_rx_valid=scales.x>=0.0;
    gi_rx_normal=unit_normal(gi_bin_source(gi_viewed_normal(input.receiver.world,input.receiver.geometric),input.normal));
    gi_rx_sky[0u]=vec3<f32>(unpack4x8snorm(a.z).x,unpack4x8snorm(a.z).y,unpack4x8snorm(a.z).z)*scales.x;
    gi_rx_sky[1u]=vec3<f32>(unpack4x8snorm(a.z).w,unpack4x8snorm(a.w).x,unpack4x8snorm(a.w).y)*scales.x;
    gi_rx_sky[2u]=vec3<f32>(unpack4x8snorm(a.w).z,unpack4x8snorm(a.w).w,unpack4x8snorm(b.x).x)*scales.x;
    gi_rx_sky[3u]=vec3<f32>(unpack4x8snorm(b.x).y,unpack4x8snorm(b.x).z,unpack4x8snorm(b.x).w)*scales.x;
    gi_rx_sky[4u]=vec3<f32>(unpack4x8snorm(b.y).x,unpack4x8snorm(b.y).y,unpack4x8snorm(b.y).z)*scales.x;
    gi_rx_sky[5u]=vec3<f32>(unpack4x8snorm(b.y).w,unpack4x8snorm(b.z).x,unpack4x8snorm(b.z).y)*scales.x;
    gi_rx_sky[6u]=vec3<f32>(unpack4x8snorm(b.z).z,unpack4x8snorm(b.z).w,unpack4x8snorm(b.w).x)*scales.x;
    gi_rx_sky[7u]=vec3<f32>(unpack4x8snorm(b.w).y,unpack4x8snorm(b.w).z,unpack4x8snorm(b.w).w)*scales.x;
    gi_rx_sky[8u]=vec3<f32>(unpack4x8snorm(c.x).x,unpack4x8snorm(c.x).y,unpack4x8snorm(c.x).z)*scales.x;
    gi_rx_bounce[0u]=vec3<f32>(unpack4x8snorm(c.x).w,unpack4x8snorm(c.y).x,unpack4x8snorm(c.y).y)*scales.y;
    gi_rx_bounce[1u]=vec3<f32>(unpack4x8snorm(c.y).z,unpack4x8snorm(c.y).w,unpack4x8snorm(c.z).x)*scales.y;
    gi_rx_bounce[2u]=vec3<f32>(unpack4x8snorm(c.z).y,unpack4x8snorm(c.z).z,unpack4x8snorm(c.z).w)*scales.y;
    gi_rx_bounce[3u]=vec3<f32>(unpack4x8snorm(c.w).x,unpack4x8snorm(c.w).y,unpack4x8snorm(c.w).z)*scales.y;
}
// ---- GI receiver compute entry ----
// The prepared lookup (gi_rx_*) as pixel `at`'s 48-byte record.
fn gi_rx_store_at(at:u32) {
    var sky=0.0;var bounce=0.0;
    sky=max(sky,max(max(abs(gi_rx_sky[0u]).x,abs(gi_rx_sky[0u]).y),abs(gi_rx_sky[0u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[1u]).x,abs(gi_rx_sky[1u]).y),abs(gi_rx_sky[1u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[2u]).x,abs(gi_rx_sky[2u]).y),abs(gi_rx_sky[2u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[3u]).x,abs(gi_rx_sky[3u]).y),abs(gi_rx_sky[3u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[4u]).x,abs(gi_rx_sky[4u]).y),abs(gi_rx_sky[4u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[5u]).x,abs(gi_rx_sky[5u]).y),abs(gi_rx_sky[5u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[6u]).x,abs(gi_rx_sky[6u]).y),abs(gi_rx_sky[6u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[7u]).x,abs(gi_rx_sky[7u]).y),abs(gi_rx_sky[7u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[8u]).x,abs(gi_rx_sky[8u]).y),abs(gi_rx_sky[8u]).z));
    bounce=max(bounce,max(max(abs(gi_rx_bounce[0u]).x,abs(gi_rx_bounce[0u]).y),abs(gi_rx_bounce[0u]).z));
    bounce=max(bounce,max(max(abs(gi_rx_bounce[1u]).x,abs(gi_rx_bounce[1u]).y),abs(gi_rx_bounce[1u]).z));
    bounce=max(bounce,max(max(abs(gi_rx_bounce[2u]).x,abs(gi_rx_bounce[2u]).y),abs(gi_rx_bounce[2u]).z));
    bounce=max(bounce,max(max(abs(gi_rx_bounce[3u]).x,abs(gi_rx_bounce[3u]).y),abs(gi_rx_bounce[3u]).z));
    let ss=max(sky,1e-30);let bs=max(bounce,1e-30);
    let q0=pack4x8snorm(vec4<f32>(gi_rx_sky[0u].x,gi_rx_sky[0u].y,gi_rx_sky[0u].z,gi_rx_sky[1u].x)/vec4<f32>(ss,ss,ss,ss));
    let q1=pack4x8snorm(vec4<f32>(gi_rx_sky[1u].y,gi_rx_sky[1u].z,gi_rx_sky[2u].x,gi_rx_sky[2u].y)/vec4<f32>(ss,ss,ss,ss));
    let q2=pack4x8snorm(vec4<f32>(gi_rx_sky[2u].z,gi_rx_sky[3u].x,gi_rx_sky[3u].y,gi_rx_sky[3u].z)/vec4<f32>(ss,ss,ss,ss));
    let q3=pack4x8snorm(vec4<f32>(gi_rx_sky[4u].x,gi_rx_sky[4u].y,gi_rx_sky[4u].z,gi_rx_sky[5u].x)/vec4<f32>(ss,ss,ss,ss));
    let q4=pack4x8snorm(vec4<f32>(gi_rx_sky[5u].y,gi_rx_sky[5u].z,gi_rx_sky[6u].x,gi_rx_sky[6u].y)/vec4<f32>(ss,ss,ss,ss));
    let q5=pack4x8snorm(vec4<f32>(gi_rx_sky[6u].z,gi_rx_sky[7u].x,gi_rx_sky[7u].y,gi_rx_sky[7u].z)/vec4<f32>(ss,ss,ss,ss));
    let q6=pack4x8snorm(vec4<f32>(gi_rx_sky[8u].x,gi_rx_sky[8u].y,gi_rx_sky[8u].z,gi_rx_bounce[0u].x)/vec4<f32>(ss,ss,ss,bs));
    let q7=pack4x8snorm(vec4<f32>(gi_rx_bounce[0u].y,gi_rx_bounce[0u].z,gi_rx_bounce[1u].x,gi_rx_bounce[1u].y)/vec4<f32>(bs,bs,bs,bs));
    let q8=pack4x8snorm(vec4<f32>(gi_rx_bounce[1u].z,gi_rx_bounce[2u].x,gi_rx_bounce[2u].y,gi_rx_bounce[2u].z)/vec4<f32>(bs,bs,bs,bs));
    let q9=pack4x8snorm(vec4<f32>(gi_rx_bounce[3u].x,gi_rx_bounce[3u].y,gi_rx_bounce[3u].z,0.0)/vec4<f32>(bs,bs,bs,bs));
    gi_prepared[at].r0=vec4<u32>(bitcast<vec2<u32>>(vec2<f32>(select(-1.0,sky,gi_rx_valid),bounce)),q0,q1);
    gi_prepared[at].r1=vec4<u32>(q2,q3,q4,q5);
    gi_prepared[at].r2=vec4<u32>(q6,q7,q8,q9);
}
@compute @workgroup_size(8,4) fn gi_receiver_pass(@builtin(global_invocation_id) grid:vec3<u32>) {
    let id=grid.xy+primary_region.xy;
    let size=textureDimensions(primary_hit);
    if any(id>=size) || any(id>=primary_region.zw) {return;}
    if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u {return;}
    let record=textureLoad(primary_hit,vec2<i32>(id),0);
    if record.x==0u {return;}
    let hit=primary_decode(record);
    let a=primary_input(hit.batch,hit.ia,hit.placement);
    let b=primary_input(hit.batch,hit.ib,hit.placement);
    let c=primary_input(hit.batch,hit.ic,hit.placement);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    var input:SurfaceInput;
    input.receiver.world=world_position(receiver_point(a.position,b.position,c.position,w),hit.placement).xyz;
    input.receiver.geometric=primary_geometric(a.position,b.position,c.position,hit.placement);
    input.normal=vertex_output(a).normal*w.x+vertex_output(b).normal*w.y+vertex_output(c).normal*w.z;
    gi_prepare(input);
    gi_rx_store_at(id.y*size.x+id.x);
}

// ---- final gather (RENDERLAB_FINAL_GATHER, src/gi_receiver_pass.rs) ----
// Pixels no longer read the lighting cache's cells. A screen probe per FG_TILE x FG_TILE tile,
// on the surface under a covered pixel the frame picks, traces FG_RAYS rays over its
// hemisphere (hardware rays or the software trace: the module's rl_cast) and shades each hit
// with the program, the cache read at the hit (so cells reach the screen only through a
// hemisphere integral) and its own direct light; misses see the sky. The probe keeps the
// radiance as L2 SH in lux, accumulated over frames while its new point stays on its plane.
// Each pixel then takes the four probes around it, weighted by position, plane distance and
// normal, and writes the same record the receivers pass wrote (sky slots absolute:
// gi_api.wgsl GI_SKY_ABSOLUTE).
const FG_TILE:u32=16u;
const FG_RAYS:u32=16u;
const FG_HISTORY:f32=16.0;
// The radiance SH (27 floats and a pad), the probe point (w: frames of history, 0 when
// invalid) and normal (w: the tile footprint in metres). Probes live after the pixels'
// records in gi_prepared, three records a probe (src/gi_receiver_pass.rs sizes it).
struct FgProbe { sh:array<vec4<f32>,7>, point:vec4<f32>, normal:vec4<f32> };
fn fg_at(index:u32,size:vec2<u32>)->u32 { return size.x*size.y+3u*index; }
fn fg_load(index:u32,size:vec2<u32>)->FgProbe {
    let at=fg_at(index,size);
    let a=gi_prepared[at];let b=gi_prepared[at+1u];let c=gi_prepared[at+2u];
    return FgProbe(array<vec4<f32>,7>(bitcast<vec4<f32>>(a.r0),bitcast<vec4<f32>>(a.r1),bitcast<vec4<f32>>(a.r2),
        bitcast<vec4<f32>>(b.r0),bitcast<vec4<f32>>(b.r1),bitcast<vec4<f32>>(b.r2),bitcast<vec4<f32>>(c.r0)),
        bitcast<vec4<f32>>(c.r1),bitcast<vec4<f32>>(c.r2));
}
fn fg_store(index:u32,size:vec2<u32>,p:FgProbe) {
    let at=fg_at(index,size);
    gi_prepared[at]=GiPrepared(bitcast<vec4<u32>>(p.sh[0]),bitcast<vec4<u32>>(p.sh[1]),bitcast<vec4<u32>>(p.sh[2]));
    gi_prepared[at+1u]=GiPrepared(bitcast<vec4<u32>>(p.sh[3]),bitcast<vec4<u32>>(p.sh[4]),bitcast<vec4<u32>>(p.sh[5]));
    gi_prepared[at+2u]=GiPrepared(bitcast<vec4<u32>>(p.sh[6]),bitcast<vec4<u32>>(p.point),bitcast<vec4<u32>>(p.normal));
}
var<workgroup> fg_lane_sh:array<array<f32,27>,FG_RAYS>;

fn fg_hash(x:u32)->u32 {
    var h=x*0x9e3779b1u;h^=h>>16u;h*=0x85ebca6bu;h^=h>>13u;h*=0xc2b2ae35u;h^=h>>16u;return h;
}
// A covered pixel's receiver for the gather: point, viewed geometric normal, shading normal,
// trace epsilon and placement + 1 (0: none).
struct FgSurface { world:vec3<f32>, normal:vec3<f32>, shading:vec3<f32>, epsilon:f32, placement:u32 };
fn fg_surface(id:vec2<u32>,size:vec2<u32>)->FgSurface {
    var s:FgSurface;
    if any(id>=size) || any(id>=primary_region.zw) || any(id<primary_region.xy) {return s;}
    if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u {return s;}
    let record=textureLoad(primary_hit,vec2<i32>(id),0);
    if record.x==0u {return s;}
    let hit=primary_decode(record);
    let a=primary_input(hit.batch,hit.ia,hit.placement);
    let b=primary_input(hit.batch,hit.ib,hit.placement);
    let c=primary_input(hit.batch,hit.ic,hit.placement);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    s.world=world_position(receiver_point(a.position,b.position,c.position,w),hit.placement).xyz;
    let g=unit_normal(primary_geometric(a.position,b.position,c.position,hit.placement));
    s.normal=select(-g,g,dot(g,lighting.eye-s.world)>=0.0);
    let n=vertex_output(a).normal*w.x+vertex_output(b).normal*w.y+vertex_output(c).normal*w.z;
    s.shading=select(s.normal,unit_normal(n),dot(n,n)>1e-12);
    if dot(s.shading,s.normal)<0.0 {s.shading=-s.shading;}
    s.epsilon=primary_epsilon(array<vec3<f32>,3>(a.position,b.position,c.position),hit.placement);
    s.placement=record.x;
    return s;
}
fn fg_basis(n:vec3<f32>)->mat3x3<f32> {
    let s=select(-1.0,1.0,n.z>=0.0);
    let a=-1.0/(s+n.z);let b=n.x*n.y*a;
    return mat3x3<f32>(vec3<f32>(1.0+s*n.x*n.x*a,s*b,-s*n.x),vec3<f32>(b,s+n.y*n.y*a,-n.y),n);
}
// Incoming radiance (unexposed) along d from `origin`.
fn fg_radiance(origin:vec3<f32>,d:vec3<f32>,epsilon:f32,pixel:vec2<u32>)->vec3<f32> {
    let inverse=1.0/max(lighting.exposure,1e-30);
    let traced=rl_cast(origin,d,epsilon);
    if traced.record.x==0u {return rl_sky(d,1.0)*inverse;}
    let h=rl_hit_input(traced.record,origin,d,0.0,0.0,pixel);
    if !h.visible {return vec3<f32>(0.0);}
    rl_hit_lights_prepare(h.input.world);
    rl_load_parameters(h.batch);
    gi_prepare(h.input);
    rl_reflection_hit=true;
    let hs=surface(h.input);
    let radiance=rl_finite(shade(hs,h.input))*inverse;
    rl_reflection_hit=false;
    rl_hit_lights_clear();
    return radiance;
}
@compute @workgroup_size(16) fn fg_trace(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let size=textureDimensions(primary_hit);
    let tiles=(size+vec2<u32>(FG_TILE-1u))/FG_TILE;
    let index=group.y*tiles.x+group.x;
    let frame=primary_frame.sided.y;
    // The probe: the first covered pixel of four the frame picks in the tile (all lanes alike).
    var s:FgSurface;
    var pixel=vec2<u32>(0u);
    for(var k=0u;k<4u;k+=1u) {
        let h=fg_hash(index*4u+k+frame*0x632be5abu);
        let q=group.xy*FG_TILE+vec2<u32>(h%FG_TILE,(h>>8u)%FG_TILE);
        s=fg_surface(q,size);
        if s.placement!=0u {pixel=q;break;}
    }
    let found=s.placement!=0u;
    var radiance=vec3<f32>(0.0);
    var d=vec3<f32>(0.0,0.0,1.0);
    if found {
        // Stratified uniform hemisphere directions, turned each frame.
        let r=fg_hash(index^(frame*0x27d4eb2fu));
        let jitter=vec2<f32>(f32(r&0xffffu),f32(r>>16u))*(1.0/65536.0);
        let u=fract((vec2<f32>(f32(lane%4u),f32(lane/4u))+jitter)/4.0);
        let z=u.x;let rr=sqrt(max(1.0-z*z,0.0));let phi=6.283185307179586*u.y;
        d=fg_basis(s.normal)*vec3<f32>(rr*cos(phi),rr*sin(phi),z);
        let origin=fma(s.normal,vec3<f32>(s.epsilon),s.world);
        radiance=fg_radiance(origin,d,s.epsilon,pixel);
    }
    gi_sh_basis(d);
    for(var i=0u;i<9u;i+=1u) {
        fg_lane_sh[lane][i*3u]=radiance.x*gi_y[i];fg_lane_sh[lane][i*3u+1u]=radiance.y*gi_y[i];fg_lane_sh[lane][i*3u+2u]=radiance.z*gi_y[i];
    }
    workgroupBarrier();
    if lane!=0u || any(group.xy>=tiles) {return;}
    if !found {
        var none:FgProbe;
        fg_store(index,size,none);
        return;
    }
    var sh:array<f32,28>;
    for(var l=0u;l<FG_RAYS;l+=1u) {
        for(var i=0u;i<27u;i+=1u) {sh[i]+=fg_lane_sh[l][i];}
    }
    // Uniform hemisphere: pdf 1/(2 pi).
    let scale=6.283185307179586/f32(FG_RAYS);
    // The tile footprint at the probe: FG_TILE pixels of the camera angle at its distance.
    let ray=primary_ray(pixel,size);
    let pixel_angle=length(ray.dx_far-ray.dx_near)/max(length(ray.far-ray.near),1e-30);
    let footprint=f32(FG_TILE)*pixel_angle*length(s.world-lighting.eye);
    let old=fg_load(index,size);
    var count=1.0;
    if old.point.w>0.0 && dot(old.normal.xyz,s.normal)>0.9 && abs(dot(old.normal.xyz,s.world-old.point.xyz))<0.25*footprint {
        count=min(old.point.w+1.0,FG_HISTORY);
    }
    let blend=1.0/count;
    var out:FgProbe;
    for(var v=0u;v<7u;v+=1u) {
        let fresh=vec4<f32>(sh[v*4u],sh[v*4u+1u],sh[v*4u+2u],sh[v*4u+3u])*scale;
        out.sh[v]=select(fresh,mix(old.sh[v],fresh,blend),count>1.0);
    }
    out.point=vec4<f32>(s.world,count);
    out.normal=vec4<f32>(s.normal,footprint);
    fg_store(index,size,out);
}
@compute @workgroup_size(8,4) fn fg_resolve(@builtin(global_invocation_id) grid:vec3<u32>) {
    let id=grid.xy+primary_region.xy;
    let size=textureDimensions(primary_hit);
    let s=fg_surface(id,size);
    if s.placement==0u {return;}
    let tiles=(size+vec2<u32>(FG_TILE-1u))/FG_TILE;
    let f=(vec2<f32>(id)+vec2<f32>(0.5))/f32(FG_TILE)-vec2<f32>(0.5);
    let base=vec2<i32>(floor(f));
    let fr=f-floor(f);
    var sh:array<vec4<f32>,7>;
    var total=0.0;
    for(var k=0u;k<4u;k+=1u) {
        let o=vec2<i32>(i32(k&1u),i32(k>>1u));
        let t=clamp(base+o,vec2<i32>(0),vec2<i32>(tiles)-vec2<i32>(1));
        let p=fg_load(u32(t.y)*tiles.x+u32(t.x),size);
        if p.point.w<=0.0 {continue;}
        let bilinear=select(1.0-fr.x,fr.x,o.x!=0)*select(1.0-fr.y,fr.y,o.y!=0);
        let plane=abs(dot(p.normal.xyz,s.world-p.point.xyz))/max(0.1*p.normal.w,1e-6);
        let facing=max(dot(p.normal.xyz,s.normal),0.0);
        let w=max(bilinear,1e-3)*exp(-plane*plane)*facing*facing*facing*facing;
        if w<=1e-6 {continue;}
        for(var v=0u;v<7u;v+=1u) {sh[v]+=w*p.sh[v];}
        total+=w;
    }
    if total<=1e-4 {
        // No probe on this surface yet: the cache, as before (v1).
        var input:SurfaceInput;
        input.receiver.world=s.world;
        input.receiver.geometric=s.normal;
        input.normal=s.shading;
        gi_prepare(input);
        gi_rx_store_at(id.y*size.x+id.x);
        return;
    }
    let inv=1.0/total;
    for(var i=0u;i<9u;i+=1u) {
        let a=i*3u;
        gi_rx_sky[i]=vec3<f32>(sh[a/4u][a%4u],sh[(a+1u)/4u][(a+1u)%4u],sh[(a+2u)/4u][(a+2u)%4u])*inv;
    }
    for(var i=0u;i<4u;i+=1u) {gi_rx_bounce[i]=vec3<f32>(0.0);}
    gi_rx_valid=true;
    gi_rx_normal=s.shading;
    gi_rx_store_at(id.y*size.x+id.x);
}
