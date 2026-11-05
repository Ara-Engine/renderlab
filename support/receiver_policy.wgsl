// R1 test only. World-space hit vertices define the precision scale.
fn receiver_epsilon(a:vec3<f32>,b:vec3<f32>,c:vec3<f32>)->f32 {
    let extent=max(abs(a),max(abs(b),abs(c)));
    let m=max(extent.x,max(extent.y,extent.z));
    let ulp=bitcast<f32>(bitcast<u32>(m)+1u)-m;
    return max(1e-5,16.0*ulp);
}
