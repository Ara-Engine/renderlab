// RT1d: one fixed non-separable 5x5 gather; no rotated taps or adaptive radius.
// The sole signal is current sun visibility. Guides are current instance/material,
// geometric normal, plane distance and camera depth. See docs/rt1d-shadow-filter.md.
// primary_frame.sided.z enables this only for traced Interactive frames.
fn rl_shadow_visibility(light:Light,receiver:Receiver,l:vec3<f32>)->f32 {
    let centre=rl_shadow_sample(light,receiver,l);
    let record=textureLoad(primary_hit,receiver.pixel,0);
    let hit=primary_decode(record);
    let a=primary_input(hit.batch,hit.ia,hit.placement).position;
    let b=primary_input(hit.batch,hit.ib,hit.placement).position;
    let c=primary_input(hit.batch,hit.ic,hit.placement).position;
    let normal=primary_geometric(a,b,c,hit.placement);
    let depth=(camera.view_projection*vec4<f32>(receiver.world,1.0)).w;
    let sigma=0.002*max(depth,0.1)+0.0005;
    let size=vec2<i32>(textureDimensions(primary_hit));
    var total=0.0;var norm=0.0;
    for(var y=-2;y<=2;y+=1) {
        for(var x=-2;x<=2;x+=1) {
            let p=receiver.pixel+vec2<i32>(x,y);
            if any(p<vec2<i32>(0)) || any(p>=size) {continue;}
            let id=vec2<u32>(p);let scene=primary_frame.scene;
            if any(id<scene.xy) || any(id>=scene.zw) {continue;}
            if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u {continue;}
            let other_record=textureLoad(primary_hit,p,0);
            // A placement has one material/program: equality enforces both IDs.
            if other_record.x!=record.x {continue;}
            let other=primary_decode(other_record);
            let oa=primary_input(other.batch,other.ia,other.placement).position;
            let ob=primary_input(other.batch,other.ib,other.placement).position;
            let oc=primary_input(other.batch,other.ic,other.placement).position;
            let w=vec3<f32>(1.0-other.bary.x-other.bary.y,other.bary);
            let world=world_position(receiver_point(oa,ob,oc,w),other.placement).xyz;
            let n=primary_geometric(oa,ob,oc,other.placement);
            let ndot=clamp(dot(normal,n),0.0,1.0);
            let plane=abs(dot(world-receiver.world,normal));
            let weight=pow(ndot,32.0)*exp(-plane/sigma)*exp(-f32(x*x+y*y)*0.5);
            let other_receiver=primary_receiver(world,array<vec3<f32>,3>(oa,ob,oc),other.placement,p);
            total+=weight*rl_shadow_sample(light,other_receiver,l);
            norm+=weight;
        }
    }
    return select(centre,total/max(norm,1e-20),norm>0.0);
}
