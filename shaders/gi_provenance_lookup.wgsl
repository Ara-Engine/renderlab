@group(0) @binding(80) var<storage,read_write> gp_log:array<atomic<u32>>;
var<private> gp_pixel:vec2<u32>;
var<private> gp_enabled:bool;
var<private> gp_slot:u32;
var<private> gp_level:u32;
var<private> gp_bin:u32;
fn gp_begin(s:SurfaceInput){gp_pixel=s.pixel;gp_enabled=atomicLoad(&gp_log[1])!=0u && ((all(s.pixel>=vec2<u32>(1300u,640u))&&all(s.pixel<vec2<u32>(1560u,900u)))||(all(s.pixel>=vec2<u32>(1510u,900u))&&all(s.pixel<vec2<u32>(1920u,1080u))));}
fn gp_record(kind:u32,data:vec4<u32>){if !gp_enabled{return;}let i=atomicAdd(&gp_log[4],1u);if i>=2097152u{atomicStore(&gp_log[5],1u);return;}let a=16u+1048576u*4u+i*16u;atomicStore(&gp_log[a],kind);atomicStore(&gp_log[a+1u],gp_pixel.x);atomicStore(&gp_log[a+2u],gp_pixel.y);atomicStore(&gp_log[a+3u],atomicLoad(&gp_log[0]));for(var k=0u;k<4u;k+=1u){atomicStore(&gp_log[a+4u+k],data[k]);}}
fn gp_prepared(){if !gp_enabled{return;}for(var j=0u;j<4u;j+=1u){gp_record(10u+j,vec4<u32>(bitcast<vec3<u32>>(gi_rx_bounce[j]),select(0u,1u,gi_rx_valid)));}}
