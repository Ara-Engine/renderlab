// PP1: all filters read reconstructed HDR, never temporal history.
struct Settings {
    bloom_motion:vec4<f32>, // energy, half shutter fraction, max velocity, frame
    lens:vec4<f32>, // focus m, thin-lens CoC scale in pixels, max CoC, DOF enabled
    grade:vec4<f32>, // contrast, saturation, vignette, grain
    white:vec4<f32>, // RGB white-balance gains, tonemapper (0 Reinhard, 1 Filmic, 2 AgX)
    display:vec4<f32>, // aberration px, LUT strength, LUT size, unused
    extra:vec4<f32>, // bloom_threshold, anti_aliasing, sharpening, unused
};
@group(0) @binding(0) var input_colour:texture_2d<f32>;
@group(0) @binding(1) var second_colour:texture_2d<f32>;
@group(0) @binding(2) var auxiliary:texture_2d<f32>;
@group(0) @binding(3) var velocity_tiles:texture_2d<f32>;
@group(0) @binding(4) var output_colour:texture_storage_2d<rgba16float,write>;
@group(0) @binding(5) var<uniform> settings:Settings;
@group(0) @binding(6) var linear_sampler:sampler;

fn load(t:texture_2d<f32>,p:vec2<i32>)->vec4<f32> {
    return textureLoad(t,clamp(p,vec2<i32>(0),vec2<i32>(textureDimensions(t))-1),0);
}
fn bilinear(t:texture_2d<f32>,uv:vec2<f32>)->vec4<f32> {
    return textureSampleLevel(t,linear_sampler,uv,0.0);
}
fn tent(t:texture_2d<f32>,uv:vec2<f32>)->vec4<f32> {
    let d=1.0/vec2<f32>(textureDimensions(t));
    var c=vec4<f32>(0.0);
    for(var y=-1;y<=1;y++) {for(var x=-1;x<=1;x++) {
        let w=f32((2-abs(x))*(2-abs(y)))/16.0;
        c+=bilinear(t,uv+vec2<f32>(f32(x),f32(y))*d)*w;
    }}
    return c;
}
fn bloom_prefilter(rgb:vec3<f32>,thresh:f32)->vec3<f32> {
    let c=max(rgb,vec3<f32>(0.0));
    if thresh<=0.0 {return c;}
    let br=max(c.r,max(c.g,c.b));
    let knee=thresh*0.5;
    let soft=clamp(br-thresh+knee,0.0,2.0*knee);
    let contrib=max(br-thresh,(soft*soft)/max(4.0*knee,1e-5))/max(br,1e-5);
    return c*contrib;
}
fn karis_weight(c:vec3<f32>)->f32 {
    return 1.0/(1.0+dot(max(c,vec3<f32>(0.0)),vec3<f32>(0.2126,0.7152,0.0722)));
}
@compute @workgroup_size(8,8) fn downsample(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(output_colour);if any(id.xy>=size) {return;}
    let in_size=textureDimensions(input_colour);
    let d=1.0/vec2<f32>(in_size);
    let uv=(vec2<f32>(id.xy)+0.5)/vec2<f32>(size);
    let mip0=all(in_size==textureDimensions(second_colour));
    // 13-tap Jimenez downsample with Karis firefly weighting + soft-knee highlight threshold on mip 0.
    let a=bilinear(input_colour,uv+vec2<f32>(-2.0,-2.0)*d).rgb;
    let b=bilinear(input_colour,uv+vec2<f32>( 0.0,-2.0)*d).rgb;
    let c=bilinear(input_colour,uv+vec2<f32>( 2.0,-2.0)*d).rgb;
    let e=bilinear(input_colour,uv+vec2<f32>(-1.0,-1.0)*d).rgb;
    let f=bilinear(input_colour,uv+vec2<f32>( 1.0,-1.0)*d).rgb;
    let g=bilinear(input_colour,uv+vec2<f32>(-2.0, 0.0)*d).rgb;
    let h=bilinear(input_colour,uv).rgb;
    let i=bilinear(input_colour,uv+vec2<f32>( 2.0, 0.0)*d).rgb;
    let j=bilinear(input_colour,uv+vec2<f32>(-1.0, 1.0)*d).rgb;
    let k=bilinear(input_colour,uv+vec2<f32>( 1.0, 1.0)*d).rgb;
    let l=bilinear(input_colour,uv+vec2<f32>(-2.0, 2.0)*d).rgb;
    let m=bilinear(input_colour,uv+vec2<f32>( 0.0, 2.0)*d).rgb;
    let n=bilinear(input_colour,uv+vec2<f32>( 2.0, 2.0)*d).rgb;
    if mip0 {
        let th=settings.extra.x;
        let q0=(bloom_prefilter(e,th)+bloom_prefilter(f,th)+bloom_prefilter(j,th)+bloom_prefilter(k,th))*0.25;
        let q1=(bloom_prefilter(a,th)+bloom_prefilter(b,th)+bloom_prefilter(g,th)+bloom_prefilter(h,th))*0.25;
        let q2=(bloom_prefilter(b,th)+bloom_prefilter(c,th)+bloom_prefilter(h,th)+bloom_prefilter(i,th))*0.25;
        let q3=(bloom_prefilter(g,th)+bloom_prefilter(h,th)+bloom_prefilter(l,th)+bloom_prefilter(m,th))*0.25;
        let q4=(bloom_prefilter(h,th)+bloom_prefilter(i,th)+bloom_prefilter(m,th)+bloom_prefilter(n,th))*0.25;
        let w0=0.5*karis_weight(q0);
        let w1=0.125*karis_weight(q1);
        let w2=0.125*karis_weight(q2);
        let w3=0.125*karis_weight(q3);
        let w4=0.125*karis_weight(q4);
        let rgb=(q0*w0+q1*w1+q2*w2+q3*w3+q4*w4)/max(w0+w1+w2+w3+w4,1e-5);
        textureStore(output_colour,id.xy,vec4<f32>(min(rgb,vec3<f32>(65504.0)),1.0));
        return;
    }
    let rgb=(e+f+j+k)*0.125+(a+c+l+n)*0.03125+(b+g+i+m)*0.0625+h*0.125;
    textureStore(output_colour,id.xy,vec4<f32>(min(max(rgb,vec3<f32>(0.0)),vec3<f32>(65504.0)),1.0));
}
@compute @workgroup_size(8,8) fn upsample(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(output_colour);if any(id.xy>=size) {return;}
    let uv=(vec2<f32>(id.xy)+0.5)/vec2<f32>(size);
    // Convex combination preserves DC at every level: no additive energy gain.
    textureStore(output_colour,id.xy,mix(load(input_colour,vec2<i32>(id.xy)),tent(second_colour,uv),0.65));
}
@compute @workgroup_size(8,8) fn tile_max(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(output_colour);if any(id.xy>=size) {return;}
    var best=vec2<f32>(0.0);
    for(var y=0;y<16;y++) {for(var x=0;x<16;x++) {
        let v=load(auxiliary,vec2<i32>(id.xy*16u)+vec2<i32>(x,y)).xy;
        if dot(v,v)>dot(best,best) {best=v;}
    }}
    textureStore(output_colour,id.xy,vec4<f32>(best,0.0,0.0));
}
fn cone(d:f32,r:f32)->f32 {return clamp(1.0-d/max(r,0.001),0.0,1.0);}
fn cylinder(d:f32,r:f32)->f32 {return 1.0-smoothstep(max(0.0,r-1.0),r+1.0,d);}
@compute @workgroup_size(8,8) fn motion_blur(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(output_colour);if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);let a=load(auxiliary,p);let own=a.xy;
    var v=own;
    let tile=p/16;
    let reach=i32(ceil(settings.bloom_motion.z/16.0));
    for(var y=-reach;y<=reach;y++) {for(var x=-reach;x<=reach;x++) {
        let candidate=load(velocity_tiles,tile+vec2<i32>(x,y)).xy;
        if dot(candidate,candidate)>dot(v,v) {v=candidate;}
    }}
    let radius=length(v);
    let centre=load(input_colour,p);
    if radius<0.5 {textureStore(output_colour,p,centre);return;}
    // McGuire et al. 2012: neighbour max direction, soft depth comparison,
    // foreground/background cones and cylinder overlap. Original implementation.
    var sum=centre.rgb/max(length(own),1.0);var weight=1.0/max(length(own),1.0);
    for(var i=0;i<16;i++) {
        let t=(f32(i)+0.5)/8.0-1.0;
        let q=vec2<f32>(p)+0.5+v*t;
        let b=load(auxiliary,vec2<i32>(q));let d=abs(t)*radius;
        let soft=max(0.02,0.02*min(a.z,b.z));
        let front=clamp(1.0-(b.z-a.z)/soft,0.0,1.0);
        let back=clamp(1.0-(a.z-b.z)/soft,0.0,1.0);
        let w=front*cone(d,length(b.xy))+back*cone(d,length(own))
            +2.0*cylinder(d,length(b.xy))*cylinder(d,length(own));
        sum+=bilinear(input_colour,q/vec2<f32>(size)).rgb*w;weight+=w;
    }
    textureStore(output_colour,p,vec4<f32>(sum/max(weight,1e-5),centre.a));
}
fn coc(z:f32)->f32 {
    return clamp(settings.lens.y*(z-settings.lens.x)/max(z,0.001),-settings.lens.z,settings.lens.z);
}
@compute @workgroup_size(8,8) fn depth_of_field(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(output_colour);if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);let z=load(auxiliary,p).z;let c=coc(z);
    let centre=load(input_colour,p);
    let foreground=c< -0.5;
    var far_sum=select(centre.rgb,vec3<f32>(0.0),foreground);
    var far_w=select(1.0,0.0,foreground);
    var near_sum=vec3<f32>(0.0);var near_w=0.0;
    let gather_radius=select(settings.lens.z,-c,foreground);
    // A fixed disc gathers both fields, including foreground bokeh beyond its silhouette.
    // Separate near/far weights stop an in-focus foreground from accepting far colour.
    for(var i=0;i<48;i++) {
        let r=sqrt((f32(i)+0.5)/48.0)*gather_radius;
        let angle=f32(i)*2.39996323;
        let q=vec2<f32>(p)+0.5+vec2<f32>(cos(angle),sin(angle))*r;
        let nz=load(auxiliary,vec2<i32>(q)).z;let nc=coc(nz);
        let colour=bilinear(input_colour,q/vec2<f32>(size)).rgb;
        if nc< -0.5 {
            let w=cylinder(r,-nc)/max(nc*nc,1.0);
            near_sum+=colour*w;near_w+=w;
        }
        if c>0.5 && nc>0.0 && nz>=z-max(0.02,z*0.02) {
            let w=cylinder(r,min(c,nc));far_sum+=colour*w;far_w+=w;
        }
        if foreground && nc>= -0.5 {
            // Estimate the background under a blurred foreground boundary. Keeping
            // the sharp foreground centre as background would leave a hard inner edge.
            let w=cylinder(r,-c);far_sum+=colour*w;far_w+=w;
        }
    }
    var far_colour=centre.rgb;
    if far_w>1e-5 {far_colour=far_sum/far_w;}
    let coverage=clamp(near_w*gather_radius*gather_radius/48.0,0.0,1.0);
    textureStore(output_colour,p,vec4<f32>(mix(far_colour,near_sum/max(near_w,1e-6),coverage),centre.a));
}
