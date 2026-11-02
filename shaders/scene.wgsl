struct Params { camera: vec4f, material: vec4f, spare: vec4f }
@group(0) @binding(0) var<uniform> p: Params;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let xy = array(vec2f(-1,-1), vec2f(3,-1), vec2f(-1,3));
    return vec4f(xy[i],0,1);
}
fn sphere(ro: vec3f, rd: vec3f, center: vec3f, radius: f32) -> f32 {
    let oc = ro-center; let b = dot(oc,rd); let d = b*b-dot(oc,oc)+radius*radius;
    if d < 0 { return 1e5; }
    let t = -b-sqrt(d); if t > 0.001 { return t; } return 1e5;
}
fn center() -> vec3f { return vec3f(-0.9,0.85,0.3 + p.material.w); }
fn visibility(pos: vec3f, light: vec3f) -> f32 {
    let delta = light-pos; let rd = normalize(delta);
    if sphere(pos,rd,center(),0.85) < length(delta) { return 0; }
    if sphere(pos,rd,vec3f(1.0,0.6,-0.4),0.6) < length(delta) { return 1-p.material.z; }
    return 1;
}
fn sample_scene(ro: vec3f, rd: vec3f) -> vec3f {
    var t = 1e5; var n = vec3f(0,1,0); var base = vec3f(0.3); var hit = false;
    if rd.y < -0.0001 { t = -ro.y/rd.y; hit = t > 0; }
    if hit {
        let q = ro+rd*t;
        let checker = (i32(floor(q.x))+i32(floor(q.z))) & 1;
        base = mix(vec3f(0.18,0.22,0.27),vec3f(0.3,0.35,0.4),f32(checker));
    }
    let st = sphere(ro,rd,center(),0.85);
    if st < t { t = st; n = normalize(ro+rd*t-center()); base = vec3f(0.75,0.16,0.055); hit = true; }
    var color = vec3f(0.055,0.085,0.14) + 0.08*max(rd.y,0);
    if hit {
        let pos = ro+rd*t; var diffuse = 0.0; var specular = 0.0;
        // Fixed area-light samples: deterministic and history-free, but not a converged reference.
        for (var y = 0; y < 4; y++) { for (var x = 0; x < 4; x++) {
            let light = vec3f(p.material.x + (f32(x)-1.5)*0.35,5,2+(f32(y)-1.5)*0.35);
            let l = normalize(light-pos); let v = visibility(pos+n*0.003,light);
            diffuse += max(dot(n,l),0)*v/16;
            specular += pow(max(dot(n,normalize(l-rd)),0),mix(160.0,4.0,p.material.y))*v/16;
        }}
        color = base*(0.10 + diffuse*1.4) + vec3f(specular*0.28);
        color = mix(color,vec3f(0.055,0.085,0.14),1-exp(-t*0.018));
    }
    // Thin tinted surface; deliberately no refraction or multiple internal intersections.
    let glass = sphere(ro,rd,vec3f(1,0.6,-0.4),0.6);
    if glass < t {
        let gn = normalize(ro+rd*glass-vec3f(1,0.6,-0.4));
        let rim = pow(1-max(dot(gn,-rd),0),3.0);
        color = mix(color,vec3f(0.08,0.65,0.7)*(0.5+0.5*gn.y),p.material.z) + 0.25*rim*p.material.z;
    }
    return color/(color+vec3f(1));
}
@fragment fn fs(@builtin(position) pixel: vec4f) -> @location(0) vec4f {
    let ro = vec3f(sin(p.camera.z)*5.5,p.camera.w,cos(p.camera.z)*5.5);
    let forward = normalize(vec3f(0,0.6,0)-ro); let right = normalize(cross(forward,vec3f(0,1,0))); let up = cross(right,forward);
    var color = vec3f(0);
    for (var y = 0; y < 2; y++) { for (var x = 0; x < 2; x++) {
        let uv = (pixel.xy+vec2f(f32(x),f32(y))*0.5-0.25)/p.camera.xy*2-1;
        let rd = normalize(forward+right*uv.x*p.camera.x/p.camera.y*0.65-up*uv.y*0.65);
        color += sample_scene(ro,rd)*0.25;
    }}
    return vec4f(color,1);
}
