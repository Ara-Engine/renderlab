// RENDERLAB_RS_ORDER=0 (src/program.rs): the BVH4 child order before the register network,
// in place of the "rs child order" block of reflections_software_bvh4.wgsl.
            var distances:array<f32,4>;
            var ids:array<u32,4>;
            for(var c=0u;c<4u;c+=1u) {
                ids[c]=children[c];
                distances[c]=3.4028235e38;
                if children[c]!=0xffffffffu {
                    let lo=vec3<f32>((lower>>vec3<u32>(c*8u))&vec3<u32>(255u))*step+n0.xyz;
                    let hi=vec3<f32>((upper>>vec3<u32>(c*8u))&vec3<u32>(255u))*step+n0.xyz;
                    distances[c]=rs_box(o,inv,lo,hi,tmin,lim);
                }
            }
            for(var c=1u;c<4u;c+=1u) {
                var j=c;
                loop {
                    if j==0u {break;}
                    if distances[j]>=distances[j-1u] {break;}
                    let t=distances[j];distances[j]=distances[j-1u];distances[j-1u]=t;
                    let id=ids[j];ids[j]=ids[j-1u];ids[j-1u]=id;
                    j-=1u;
                }
            }
            for(var c=4u;c>0u;c-=1u) {
                if distances[c-1u]<=lim && ids[c-1u]!=0xffffffffu {
                    (*stack)[sp]=ids[c-1u];(*entries)[sp]=distances[c-1u];sp+=1u;
                }
            }
            break;
