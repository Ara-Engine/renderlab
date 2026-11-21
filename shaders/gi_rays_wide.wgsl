// ---- the static world's flattened 8-wide BVH (RENDERLAB_GI_WIDE, src/gi_wide_bvh.rs) ----
// Composed after gi_rays_software.wgsl only when the switch is on; gi_traverse calls it first
// and then traces the movers' two-level structure against the hit it found.
// Nodes (20 words): origin, exponent bytes, child base, triangle base, eight slot bytes (0
// empty; 0x40|k the k-th inner child; 0x80|(count-1)<<5|offset a leaf of 1-3 triangles), then
// the children's 8-bit corners by axis (low x, y, z; high x, y, z; eight bytes each).
// Triangles (11 words): world-space corners, the instance record (bit 31: casts no shadow) and
// the word of its first position index.
// Deferred leaves share the stack with nodes: up to seven siblings a level wait on it.
const GI_WIDE_STACK:u32=96u;
const GI_WIDE_LEAF:u32=0x80000000u;
fn gi_wide_byte(w:u32, k:u32) -> u32 { return (w>>(8u*(k&3u)))&0xffu; }
// The node-level bound: one ulp above the best hit, so equal-distance ties stay visitable.
fn gi_wide_bound(best:GiHit) -> f32 { return select(best.t,bitcast<f32>(bitcast<u32>(best.t)+1u),best.hit); }
fn gi_wide_traverse(origin:vec3<f32>, direction:vec3<f32>, inv:vec3<f32>, tmin:f32, any_hit:bool, best:ptr<function,GiHit>) {
    let nodes=gi_geometry[8];
    if nodes==0u { return; }
    let tris=gi_geometry[9];
    var stack_n:array<u32,GI_WIDE_STACK>;
    var stack_t:array<f32,GI_WIDE_STACK>;
    stack_n[0]=0u;stack_t[0]=tmin;
    var sp=1u;
    // The best hit found here, finished (its position indices and normal) once at the end.
    var found=false;var found_word=0u;
    while sp>0u {
        sp-=1u;
        if stack_t[sp]>gi_wide_bound(*best) { continue; }
        let entry=stack_n[sp];
        // A leaf a closest-hit ray deferred (GI_WIDE_LEAF | first triangle << 2 | count - 1): its
        // triangles in distance order with the nodes.
        if (entry&GI_WIDE_LEAF)!=0u {
            let first=(entry&~GI_WIDE_LEAF)>>2u;
            for(var j=0u;j<(entry&3u)+1u;j+=1u) {
                let r=tris+(first+j)*11u;
                let inst=gi_geometry[r+9u]&0x7fffffffu;
                if GI_WIDE_COUNT { gi_count_tris+=1u; }
                let h=gi_triangle(origin,direction,gi_gv(r),gi_gv(r+3u),gi_gv(r+6u),tmin,gi_limit(*best,inst));
                if h.x>=0.0 {
                    *best=GiHit(true,h.x,vec3<f32>(0.0),inst,vec3<u32>(0u),h.yz);
                    found=true;found_word=gi_geometry[r+10u];
                }
            }
            continue;
        }
        if GI_WIDE_COUNT { gi_count_nodes+=1u; }
        let n=nodes+entry*20u;
        let p=gi_gv(n);
        let ew=gi_geometry[n+3u];
        let scale=vec3<f32>(bitcast<f32>((ew&0xffu)<<23u),bitcast<f32>(((ew>>8u)&0xffu)<<23u),bitcast<f32>(((ew>>16u)&0xffu)<<23u));
        let child_base=gi_geometry[n+4u];let tri_base=gi_geometry[n+5u];
        let m0=gi_geometry[n+6u];let m1=gi_geometry[n+7u];
        // Inner children hit, kept sorted far to near, pushed after the leaves are tested.
        var cn:array<u32,8>;var ct:array<f32,8>;var count=0u;
        for(var k=0u;k<8u;k+=1u) {
            let slot=gi_wide_byte(select(m1,m0,k<4u),k);
            if slot==0u { continue; }
            let half=k/4u;
            let lq=vec3<f32>(f32(gi_wide_byte(gi_geometry[n+8u+half],k)),f32(gi_wide_byte(gi_geometry[n+10u+half],k)),f32(gi_wide_byte(gi_geometry[n+12u+half],k)));
            let hq=vec3<f32>(f32(gi_wide_byte(gi_geometry[n+14u+half],k)),f32(gi_wide_byte(gi_geometry[n+16u+half],k)),f32(gi_wide_byte(gi_geometry[n+18u+half],k)));
            let bound=gi_wide_bound(*best);
            let t=gi_slab_near(origin,inv,p+lq*scale,p+hq*scale,tmin,bound);
            if t>bound { continue; }
            if (slot&0x80u)!=0u && !any_hit {
                // Closest hit: the leaf joins the sorted children, tested when its turn comes.
                var i=count;
                count+=1u;
                while i>0u && ct[i-1u]<t { ct[i]=ct[i-1u];cn[i]=cn[i-1u];i-=1u; }
                ct[i]=t;cn[i]=GI_WIDE_LEAF|((tri_base+(slot&31u))<<2u)|((slot>>5u)&3u);
                continue;
            }
            if (slot&0x80u)!=0u {
                let first=tri_base+(slot&31u);
                let leaf=((slot>>5u)&3u)+1u;
                for(var j=0u;j<leaf;j+=1u) {
                    let r=tris+(first+j)*11u;
                    let word=gi_geometry[r+9u];
                    let inst=word&0x7fffffffu;
                    if any_hit && (word&0x80000000u)!=0u { continue; }
                    if GI_WIDE_COUNT { gi_count_tris+=1u; }
                    let h=gi_triangle(origin,direction,gi_gv(r),gi_gv(r+3u),gi_gv(r+6u),tmin,gi_limit(*best,inst));
                    if h.x>=0.0 {
                        *best=GiHit(true,h.x,vec3<f32>(0.0),inst,vec3<u32>(0u),h.yz);
                        found=true;found_word=gi_geometry[r+10u];
                        if any_hit { sp=0u;count=0u;break; }
                    }
                }
                if any_hit && found { break; }
                continue;
            }
            var i=count;
            count+=1u;
            while i>0u && ct[i-1u]<t { ct[i]=ct[i-1u];cn[i]=cn[i-1u];i-=1u; }
            ct[i]=t;cn[i]=child_base+(slot&7u);
        }
        for(var i=0u;i<count;i+=1u) {
            if sp<GI_WIDE_STACK { stack_n[sp]=cn[i];stack_t[sp]=ct[i];sp+=1u; }
        }
    }
    if found {
        // As the two-level structure reports it: the position indices in the instance's mesh and
        // the source-winding normal through the instance's world-to-object rows.
        let r=16u+(*best).inst*GI_INSTANCE_WORDS;
        let ia=gi_geometry[found_word];let ib=gi_geometry[found_word+1u];let ic=gi_geometry[found_word+2u];
        let positions=gi_geometry[r+20u];
        let a=gi_gv(positions+ia*3u);let b=gi_gv(positions+ib*3u);let c=gi_gv(positions+ic*3u);
        let r0=gi_gv(r);let r1=gi_gv(r+4u);let r2=gi_gv(r+8u);
        (*best).normal=unit_normal(mat3x3<f32>(r0,r1,r2)*cross(b-a,c-a));
        (*best).tri=vec3<u32>(ia,ib,ic);
    }
}
