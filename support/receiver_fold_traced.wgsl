fn fold_parameters(pixel:vec2<u32>,size:vec2<u32>)->vec4<f32> {
    let slope=f32(2u<<(pixel.y/6u));let facet=0.002*f32(1u<<((pixel.y/2u)%3u));
    let extent=facet/sqrt(1.0+slope*slope);
    let u=pow(10.0,fma(f32(pixel.x)/f32(size.x-1u),8.0,-8.0));
    return vec4<f32>(slope,facet,extent,u);
}
fn primary_ray(pixel:vec2<u32>,size:vec2<u32>)->PrimaryRay {
    let p=fold_parameters(pixel,size);let x=fma(p.z,p.w,0.05*f32(pixel.y));let y=-p.y*p.w;
    return PrimaryRay(vec3<f32>(x,y,0.1),vec3<f32>(x,y,-0.1),vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(0.0));
}
fn fold_neighbour(origin:vec3<f32>,direction:vec3<f32>,slope:f32,extent:f32,epsilon:f32)->bool {
    let rate=direction.z+slope*direction.x;
    let t=-(origin.z+slope*origin.x)/rate;let x=fma(direction.x,t,origin.x);
    return t>epsilon && t<1.0 && x>=-extent && x<=0.0;
}
@compute @workgroup_size(8,4) fn primary_trace(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(primary_hit_output);if any(id.xy>=size) {return;}
    let p=fold_parameters(id.xy,size);let slope=p.x;let facet=p.y;let extent=p.z;let u=p.w;
    let trace=primary_closest(id.xy,size);
    if trace.record.x==0u {textureStore(primary_hit_output,id.xy,vec4<u32>(0u,8u,0u,bitcast<u32>(u)));return;}
    let slot=trace.record.x-1u;let epsilon=primary_epsilon(trace.positions,slot);
    let n=primary_geometric(trace.positions[0],trace.positions[1],trace.positions[2],slot);
    let viewed=select(-n,n,n.z>=0.0);
    var origin=fma(viewed,vec3<f32>(epsilon),trace.point);origin.x-=0.05*f32(id.y);
    var direction=normalize(vec3<f32>(select(0.0,-0.999/slope,id.y%2u!=0u),0.0,1.0));
    var blocked_direction=normalize(vec3<f32>(-1.0,0.0,slope*0.1));
    if trace.record.y%2u!=0u {origin.x=-origin.x;direction.x=-direction.x;blocked_direction.x=-blocked_direction.x;}
    let old_point=vec3<f32>(extent*u,-facet*u,slope*extent*u);
    let old_normal=normalize(vec3<f32>(-slope,0.0,1.0));
    let old_origin=fma(old_normal,vec3<f32>(epsilon),old_point);
    let old_direction=normalize(vec3<f32>(select(0.0,-0.999/slope,id.y%2u!=0u),0.0,1.0));
    textureStore(primary_hit_output,id.xy,vec4<u32>(u32(fold_neighbour(old_origin,old_direction,slope,extent,epsilon)),u32(fold_neighbour(origin,direction,slope,extent,epsilon)),u32(fold_neighbour(origin,blocked_direction,slope,extent,epsilon)),bitcast<u32>(u)));
}
