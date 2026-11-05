@group(0) @binding(0) var output:texture_storage_2d<rgba32uint,write>;
// Independent segment-plane intersection on the neighbouring facet, extruded
// across y. All sampled rays have y=0 and stay inside that extrusion.
fn neighbour(origin:vec3<f32>,direction:vec3<f32>,slope:f32,extent:f32,epsilon:f32)->bool {
    let rate=direction.z+slope*direction.x;
    let t=-(origin.z+slope*origin.x)/rate;
    let x=fma(direction.x,t,origin.x);
    return t>epsilon && t<1.0 && x>=-extent && x<=0.0;
}
@compute @workgroup_size(64) fn fold(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(output);
    if any(id.xy>=size) {return;}
    let slope=pow(2.0,f32((id.y%18u)/6u+1u));
    let facet=0.002*pow(2.0,f32((id.y/2u)%3u));
    let extent=facet/sqrt(1.0+slope*slope);
    let a=vec3<f32>(0.0,-facet,0.0);let b=vec3<f32>(extent,-facet,slope*extent);let c=vec3<f32>(0.0,facet,0.0);
    let u=pow(10.0,fma(f32(id.x)/f32(size.x-1u),8.0,-8.0));
    let w=vec3<f32>((1.0-u)*0.5,u,(1.0-u)*0.5);
    let point=receiver_point(a,b,c,w);let epsilon=receiver_epsilon(a,b,c);
    let normal=unit_normal(cross(b-a,c-a));
    lighting.eye=vec3<f32>(0.0,0.0,1.0);
    let direction=normalize(vec3<f32>(select(0.0,-0.999/slope,id.y%2u!=0u),0.0,1.0));
    let old_origin=fma(normal,vec3<f32>(epsilon),point);
    // Exact shared-edge adjacency is supplied independently of traversal here.
    let other=vec3<f32>(-extent,-facet,slope*extent);
    let inset=receiver_fold_weights(b-a,c-a,w,1u,other-a,lighting.eye-point,epsilon);
    let receiver=Receiver(receiver_point(a,b,c,inset),normal,vec2<i32>(id.xy),0u,epsilon,id.y>=18u);
    let new_origin=shadow_origin(receiver,direction);
    let reflected=other-a-2.0*dot(normal,other-a)*normal;
    let convex=receiver_fold_weights(b-a,c-a,w,1u,reflected,lighting.eye-point,epsilon);
    let flat=receiver_fold_weights(b-a,c-a,w,1u,b-a,lighting.eye-point,epsilon);
    let invalid=u32(length(new_origin-point)>5.01*epsilon)*2u+u32(any(convex!=w) || any(flat!=w))*4u;
    // A real neighbour blocker above the grazing horizon must still intersect.
    let blocked_direction=normalize(vec3<f32>(-1.0,0.0,slope*0.1));
    let blocked_origin=shadow_origin(receiver,blocked_direction);
    textureStore(output,id.xy,vec4<u32>(u32(neighbour(old_origin,direction,slope,extent,epsilon)),u32(neighbour(new_origin,direction,slope,extent,epsilon))+invalid,u32(neighbour(blocked_origin,blocked_direction,slope,extent,epsilon)),bitcast<u32>(u)));
}
