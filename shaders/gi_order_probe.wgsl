// Measurement only. No production buffer is written by these entries.
@group(0) @binding(38) var<storage,read_write> go_rows:array<atomic<u32>>;
var<private> go_acc:array<u32,30>;
var<workgroup> go_frame:u32;
var<workgroup> go_reduce:array<u32,1920>;
fn go_mix(v:u32)->u32{var x=v;x^=x>>16u;x*=0x7feb352du;x^=x>>15u;x*=0x846ca68bu;return x^(x>>16u);}
fn go_word(h:u32,v:u32)->u32{return go_mix(h^go_mix(v+0x9e3779b9u));}
fn go_push(d:u32,h:u32){go_acc[3u*d]+=1u;go_acc[3u*d+1u]^=go_mix(h);go_acc[3u*d+2u]+=go_mix(h^0xa511e9b3u);}
fn go_scan(id:u32,lane:u32,stage:u32){
 if lane==0u{go_frame=atomicLoad(&go_rows[0]);}let frame=workgroupUniformLoad(&go_frame);if frame>=2048u{return;}
 let row=16u+(frame*8u+stage)*48u;
 for(var j=0u;j<30u;j+=1u){go_acc[j]=0u;}
 let dimensions=textureDimensions(primary_hit);let pixel_count=dimensions.x*dimensions.y;
 let misses=gi_miss_count();let slots=gi_frame.offsets.x;
 let count=max(slots,select(misses,max(misses,pixel_count),stage==0u));
 for(var i=id;i<count;i+=16384u){
  if stage==0u && i<pixel_count{let v=textureLoad(primary_hit,vec2<i32>(vec2<u32>(i%dimensions.x,i/dimensions.x)),0);var h=go_mix(i);for(var k=0u;k<4u;k+=1u){h=go_word(h,v[k]);}go_push(0u,h);}
  if (stage==1u || stage==2u) && i<misses{
   let base=gi_frame.offsets.y+i*GI_MISS_WORDS;let key=gi_miss_key(base);let h=go_word(go_word(key.x,key.y),gi_miss_band(base));
   go_push(1u,go_word(h,i));go_push(2u,h);if gi_miss_chosen(base){go_push(3u,h);}
  }
  if i>=slots{continue;}let hot=gi_hot(i);let cold=gi_cold(i);let ka=gi_table[hot];if ka==0u{continue;}
  let key=go_word(ka&GI_KEY_BITS,gi_table[cold+GI_C_KEY_B]);go_push(4u,key);
  var st=go_word(key,ka);for(var k=1u;k<max(24u,gi_word(GI_H_COLD_WORDS));k+=1u){if k!=GI_C_BASE{st=go_word(st,gi_table[cold+k]);}}go_push(5u,st);
  var sky=go_word(key,ka);for(var k=1u;k<16u;k+=1u){sky=go_word(sky,gi_table[hot+k]);}go_push(6u,sky);
  var raw=key;for(var k=16u;k<23u;k+=1u){var v=gi_table[hot+k];if gi_filter_on(GI_FILTER_BOUNCE){v=gi_raw[gi_raw_at(i,k)];}raw=go_word(raw,v);}go_push(7u,raw);
  let outgoing=bitcast<vec4<u32>>(gi_outgoing[i]);var radiance=key;for(var k=0u;k<4u;k+=1u){radiance=go_word(radiance,outgoing[k]);}go_push(8u,radiance);
  var bounce=key;for(var k=16u;k<23u;k+=1u){bounce=go_word(bounce,gi_table[hot+k]);}go_push(9u,bounce);
 }
 for(var j=0u;j<30u;j+=1u){go_reduce[lane*30u+j]=go_acc[j];}workgroupBarrier();
 for(var step=32u;step>0u;step/=2u){if lane<step{for(var j=0u;j<30u;j+=1u){let other=go_reduce[(lane+step)*30u+j];if j%3u==1u{go_reduce[lane*30u+j]^=other;}else{go_reduce[lane*30u+j]+=other;}}}workgroupBarrier();}
 if lane==0u{for(var j=0u;j<30u;j+=1u){if j%3u==1u{atomicXor(&go_rows[row+4u+j],go_reduce[j]);}else{atomicAdd(&go_rows[row+4u+j],go_reduce[j]);}}}
 if id==0u{atomicStore(&go_rows[row],1u);atomicStore(&go_rows[row+1u],gi_word(GI_H_FRAME));atomicStore(&go_rows[row+2u],atomicLoad(&gi_work[GI_W_MISSES]));atomicStore(&go_rows[row+3u],atomicLoad(&gi_work[GI_W_GATHERS]));atomicStore(&go_rows[row+34u],atomicLoad(&gi_work[GI_W_CREATED]));atomicStore(&go_rows[row+35u],atomicLoad(&gi_work[GI_W_SHADES]));atomicStore(&go_rows[row+36u],atomicLoad(&gi_work[GI_W_GATHER_RAYS]));atomicStore(&go_rows[row+37u],atomicLoad(&gi_work[GI_W_GATHER_DEFERRED]));atomicStore(&go_rows[row+38u],atomicLoad(&gi_work[GI_W_CLAIM_DEFERRED]));atomicStore(&go_rows[row+40u],gi_word(32u));atomicStore(&go_rows[row+41u],gi_word(33u));atomicStore(&go_rows[row+42u],gi_word(GI_H_PARENT_STANDIN));var uniform_hash=0u;for(var k=0u;k<64u;k+=1u){uniform_hash=go_word(uniform_hash,gi_word(k));}atomicStore(&go_rows[row+44u],uniform_hash);atomicStore(&go_rows[row+45u],dimensions.x);atomicStore(&go_rows[row+46u],dimensions.y);}
}
@compute @workgroup_size(64) fn go_stage0(@builtin(global_invocation_id)id:vec3<u32>,@builtin(local_invocation_index)lane:u32){go_scan(id.x,lane,0u);}
@compute @workgroup_size(64) fn go_stage1(@builtin(global_invocation_id)id:vec3<u32>,@builtin(local_invocation_index)lane:u32){go_scan(id.x,lane,1u);}
@compute @workgroup_size(64) fn go_stage2(@builtin(global_invocation_id)id:vec3<u32>,@builtin(local_invocation_index)lane:u32){go_scan(id.x,lane,2u);}
@compute @workgroup_size(64) fn go_stage3(@builtin(global_invocation_id)id:vec3<u32>,@builtin(local_invocation_index)lane:u32){go_scan(id.x,lane,3u);}
@compute @workgroup_size(64) fn go_stage4(@builtin(global_invocation_id)id:vec3<u32>,@builtin(local_invocation_index)lane:u32){go_scan(id.x,lane,4u);}
@compute @workgroup_size(64) fn go_stage5(@builtin(global_invocation_id)id:vec3<u32>,@builtin(local_invocation_index)lane:u32){go_scan(id.x,lane,5u);}
@compute @workgroup_size(64) fn go_stage6(@builtin(global_invocation_id)id:vec3<u32>,@builtin(local_invocation_index)lane:u32){go_scan(id.x,lane,6u);}
@compute @workgroup_size(64) fn go_stage7(@builtin(global_invocation_id)id:vec3<u32>,@builtin(local_invocation_index)lane:u32){go_scan(id.x,lane,7u);}
