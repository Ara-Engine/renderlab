// Software-only screen intersections. Zero confidence always means a complete tree fallback.
struct RlScreenFrame {
    current:mat4x4<f32>, previous:mat4x4<f32>, previous_inverse:mat4x4<f32>,
    eye:vec4<f32>, previous_eye:vec4<f32>, flags:vec4<u32>, tune:vec4<f32>,
};
@group(2) @binding(2) var<uniform> rl_screen_frame:RlScreenFrame;
// Nearest reversed depth, uniform placement/primitive (zero when mixed), farthest depth.
@group(2) @binding(3) var rl_screen_depths:texture_2d<u32>;
@group(2) @binding(4) var rl_screen_hits:texture_2d<u32>;
@group(2) @binding(5) var rl_screen_tiles:texture_2d<u32>;
@group(2) @binding(6) var rl_screen_previous:texture_2d<u32>;
@group(2) @binding(7) var rl_screen_colours:texture_2d<f32>;
@group(2) @binding(11) var rl_screen_colour_out:texture_storage_2d<rgba16float,write>;
@group(2) @binding(12) var rl_screen_depth_out:texture_storage_2d<rgba32uint,write>;

fn rl_screen_triangle(record:vec4<u32>)->array<vec3<f32>,3> {
    let h=primary_decode(record);
    return array<vec3<f32>,3>(world_position(primary_input(h.batch,h.ia,h.placement).position,h.placement).xyz,
        world_position(primary_input(h.batch,h.ib,h.placement).position,h.placement).xyz,
        world_position(primary_input(h.batch,h.ic,h.placement).position,h.placement).xyz);
}
@compute @workgroup_size(8,8) fn rl_screen_depth(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(rl_screen_depth_out);if any(id.xy>=size) {return;}
    var value=vec4<u32>(0u);let p=vec2<i32>(id.xy);
    // Primary hits outside covered tiles can retain an older frame's record.
    if textureLoad(rl_screen_tiles,p/vec2<i32>(8,4),0).x!=0u {
        let record=textureLoad(rl_screen_hits,p,0);
        if record.x!=0u {
            let tri=rl_screen_triangle(record);let b=bitcast<vec2<f32>>(record.zw);
            let point=receiver_point(tri[0],tri[1],tri[2],vec3<f32>(1.0-b.x-b.y,b));
            let clip=rl_screen_frame.current*vec4<f32>(point,1.0);
            let z=bitcast<u32>(clamp(clip.z/clip.w,0.0,1.0));
            value=vec4<u32>(z,record.xy,z);
        }
    }
    textureStore(rl_screen_depth_out,p,value);
}
@compute @workgroup_size(8,8) fn rl_screen_reduce(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(rl_screen_depth_out);if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy)*2;
    let a=textureLoad(rl_screen_depths,p,0);let b=textureLoad(rl_screen_depths,p+vec2<i32>(1,0),0);
    let c=textureLoad(rl_screen_depths,p+vec2<i32>(0,1),0);let d=textureLoad(rl_screen_depths,p+vec2<i32>(1,1),0);
    let same=all(a.yz==b.yz)&&all(a.yz==c.yz)&&all(a.yz==d.yz);
    textureStore(rl_screen_depth_out,vec2<i32>(id.xy),vec4<u32>(max(max(a.x,b.x),max(c.x,d.x)),select(vec2<u32>(0u),a.yz,same),min(min(a.w,b.w),min(c.w,d.w))));
}
@compute @workgroup_size(8,8) fn rl_screen_colour_reduce(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(rl_screen_colour_out);if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy)*2;
    let a=textureLoad(rl_screen_colours,p,0);let b=textureLoad(rl_screen_colours,p+vec2<i32>(1,0),0);
    let c=textureLoad(rl_screen_colours,p+vec2<i32>(0,1),0);let d=textureLoad(rl_screen_colours,p+vec2<i32>(1,1),0);
    textureStore(rl_screen_colour_out,vec2<i32>(id.xy),vec4<f32>((a.rgb+b.rgb+c.rgb+d.rgb)*0.25,max(max(a.a,b.a),max(c.a,d.a))));
}
fn rl_screen_colour(pixel:vec2<f32>,key:vec2<u32>,level:u32)->vec4<f32> {
    let at=pixel/exp2(f32(level))-0.5;let p=vec2<i32>(floor(at));let f=fract(at);
    let size=vec2<i32>(textureDimensions(rl_screen_previous,level));
    if any(p<vec2<i32>(0))||any(p+1>=size) {return vec4<f32>(0.0);}
    for(var y=0;y<2;y+=1) {for(var x=0;x<2;x+=1) {
        if any(textureLoad(rl_screen_previous,p+vec2<i32>(x,y),i32(level)).yz!=key) {return vec4<f32>(0.0);}
    }}
    let a=textureLoad(rl_screen_colours,p,i32(level));let b=textureLoad(rl_screen_colours,p+vec2<i32>(1,0),i32(level));
    let c=textureLoad(rl_screen_colours,p+vec2<i32>(0,1),i32(level));let d=textureLoad(rl_screen_colours,p+vec2<i32>(1,1),i32(level));
    let hint=max(max(a.a,b.a),max(c.a,d.a));
    if hint>rl_screen_frame.tune.x || hint<1.0 {return vec4<f32>(0.0);}
    return vec4<f32>(mix(mix(a.rgb,b.rgb,f.x),mix(c.rgb,d.rgb,f.x),f.y),1.0);
}
fn rl_screen_history(point:vec3<f32>,normal:vec3<f32>,key:vec2<u32>,cone:f32)->vec4<f32> {
    let clip=rl_screen_frame.previous*vec4<f32>(point,1.0);if clip.w<=0.0 {return vec4<f32>(0.0);}
    let size=vec2<f32>(rl_screen_frame.flags.yz);let ndc=clip.xyz/clip.w;
    let pixel=(ndc.xy*vec2<f32>(0.5,-0.5)+0.5)*size;
    if any(pixel<vec2<f32>(1.0))||any(pixel>=size-1.0) {return vec4<f32>(0.0);}
    let p=vec2<i32>(pixel);let old=textureLoad(rl_screen_previous,p,0);
    if any(old.yz!=key) {return vec4<f32>(0.0);}
    let old_ndc=vec3<f32>((vec2<f32>(p)+0.5)/size*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),bitcast<f32>(old.x));
    let q=rl_screen_frame.previous_inverse*vec4<f32>(old_ndc,1.0);
    if abs(dot(q.xyz/q.w-point,normal))>max(0.002,length(point-rl_screen_frame.previous_eye.xyz)*0.0001) {return vec4<f32>(0.0);}
    // Perspective footprint in pixels, with conservative surface ownership at both mips.
    let radius=cone*abs(rl_screen_frame.previous[1].y)*size.y/max(clip.w,1e-5);
    let lod=clamp(log2(max(radius,1.0)),0.0,f32(rl_screen_frame.flags.w-1u));
    let lo=u32(floor(lod));let hi=min(lo+1u,rl_screen_frame.flags.w-1u);
    let a=rl_screen_colour(pixel,key,lo);let b=rl_screen_colour(pixel,key,hi);
    if a.a==0.0||b.a==0.0 {return vec4<f32>(0.0);}
    let colour=mix(a.rgb,b.rgb,fract(lod));
    // Lit primaries include camera fog; reflected hit shading does not. Reject opaque fog.
    let scatter=atmosphere_surface(vec3<f32>(0.0),point);
    let transmission=atmosphere_surface(vec3<f32>(1.0),point)-scatter;
    if min(transmission.x,min(transmission.y,transmission.z))<rl_screen_frame.tune.y || any(colour<scatter-vec3<f32>(0.001)) {return vec4<f32>(0.0);}
    return vec4<f32>(max((colour-scatter)/transmission,vec3<f32>(0.0)),1.0);
}
fn rl_screen_hit(origin:vec3<f32>,direction:vec3<f32>,epsilon:f32,cone:vec2<f32>,receiver:vec2<u32>)->vec4<f32> {
    atomicAdd(&rl_trace_stats.screen_try,1u);
    if rl_screen_frame.flags.x==0u {return vec4<f32>(0.0);}
    atomicAdd(&rl_trace_stats.screen_valid,1u);
    let matrix=rl_screen_frame.current;let start=matrix*vec4<f32>(origin+direction*epsilon,1.0);
    let delta=matrix*vec4<f32>(direction,0.0);var distance=rl_screen_frame.tune.z;
    if delta.w<0.0 {distance=min(distance,(start.w-0.02)/(-delta.w));}
    if start.w<=0.0||distance<=epsilon {return vec4<f32>(0.0);}
    let end=start+delta*distance;let size=vec2<f32>(rl_screen_frame.flags.yz);
    let a=start.xyz/start.w;let b=end.xyz/end.w;
    let pixel=(a.xy*vec2<f32>(0.5,-0.5)+0.5)*size;
    let step=(b.xy-a.xy)*vec2<f32>(0.5,-0.5)*size;
    if max(abs(step.x),abs(step.y))<1.0 {return vec4<f32>(0.0);}
    let own=textureLoad(rl_screen_hits,vec2<i32>(receiver),0).xy;
    let max_level=i32(rl_screen_frame.flags.w)-1;var level=min(3,max_level);var s=0.00001;
    for(var iteration=0u;iteration<40u;iteration+=1u) {
        let position=pixel+step*s;
        if s>=1.0 || any(position<vec2<f32>(0.0))||any(position>=size) {break;}
        let scale=exp2(f32(level));let cell=vec2<i32>(floor(position/scale));
        if any(cell>=vec2<i32>(textureDimensions(rl_screen_depths,level))) {level=max(level-1,0);continue;}
        let boundary=(vec2<f32>(cell)+select(vec2<f32>(0.0),vec2<f32>(1.0),step>vec2<f32>(0.0)))*scale;
        var next=vec2<f32>(1e20);
        if abs(step.x)>1e-6 {next.x=(boundary.x-pixel.x)/step.x;}
        if abs(step.y)>1e-6 {next.y=(boundary.y-pixel.y)/step.y;}
        let exit_s=min(1.0,min(next.x,next.y)+0.000001);
        let depth=textureLoad(rl_screen_depths,cell,level);
        let ray_nearest=min(mix(a.z,b.z,s),mix(a.z,b.z,exit_s));
        if depth.x==0u || (depth.y!=0u&&all(depth.yz==own)) || ray_nearest>bitcast<f32>(depth.x)+1e-7 {
            s=max(exit_s,s+0.000001);level=min(level+1,max_level);continue;
        }
        if level>0 {level-=1;continue;}
        let record=textureLoad(rl_screen_hits,cell,0);
        if record.x==0u {break;}
        let info=primary_placements[record.x-1u];if (info&(1u<<29u))!=0u {break;}
        let tri=rl_screen_triangle(record);let e1=tri[1]-tri[0];let e2=tri[2]-tri[0];
        let h=cross(direction,e2);let det=dot(e1,h);if abs(det)<1e-12 {break;}
        let offset=origin-tri[0];let u=dot(offset,h)/det;let cross_offset=cross(offset,e1);
        let v=dot(direction,cross_offset)/det;let t=dot(e2,cross_offset)/det;
        if t<=epsilon || u<0.0 || v<0.0 || u+v>1.0 {break;}
        let point=origin+direction*t;let projection=matrix*vec4<f32>(point,1.0);
        let hit_pixel=(projection.xy/projection.w*vec2<f32>(0.5,-0.5)+0.5)*size;
        // Exact triangle refinement must land in the depth cell that bracketed it.
        if any(vec2<i32>(floor(hit_pixel))!=cell) {break;}
        let normal=normalize(cross(e1,e2));
        if (info&(1u<<30u))!=0u && dot(normal,direction)>=0.0 {break;}
        if dot(normal,direction)*dot(normal,rl_screen_frame.previous_eye.xyz-point)>=0.0 {break;}
        atomicAdd(&rl_trace_stats.screen_candidate,1u);
        let colour=rl_screen_history(point,normal,record.xy,cone.x+cone.y*t);
        if colour.a>0.0 {atomicAdd(&rl_trace_stats.screen_hit,1u);}
        return colour;
    }
    return vec4<f32>(0.0);
}
