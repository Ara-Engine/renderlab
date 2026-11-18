// ---- renderlab world radiance cache: accumulation (RENDERLAB_GI_ACCUMULATE=1) ----
// docs/design-gi-accumulate.md. Composed after gi_cache.wgsl (src/gi_cache.rs kernel_source).
// A cell's value was the sum of one fixed set of 64 directions, replaced whole by every gather,
// and a small bright source (Outpost's hangar doorway: sky and sunlit ground) was met by some
// cells' fixed directions and missed by their neighbours': a fixed mottle a cell wide. Here a
// cell's SH words hold its whole incident radiance (sky and bounce, absolute), and each update
// traces a few rays in a fresh rotation and blends them into that history, so a cell converges
// on all directions over its updates; no update is an event, and a fixed budget of updates runs
// every frame whatever the camera does.
// - gi_acc_select lists this frame's updates: a cell with no history yet (finish just gave it its
//   first value, sky only), visible cells every GI_H_ACC_VISIBLE frames, others every
//   GI_H_ACC_OTHER, each at a phase its slot fixes (the integrate queue carries the list; the
//   integrate pass does not run while accumulating);
// - gi_accumulate, one workgroup per listed cell: GI_H_ACC_RAYS rays (all 64 lanes for a cell's
//   first update), stratified over the hemisphere's solid angle in a rotation drawn per update; a
//   miss adds the sky's radiance, a hit the hit cell's outgoing radiance (gi_hit_radiance, which
//   also requests the hit cell as gathers do). The L2 SH of the sum blends into the history:
//   the slow weight is 1 / updates (at most GI_H_ACC_HISTORY); a fast mean of the update's L0
//   luminance (weight 1/2) leaving the slow mean's band (twice its standard deviation) pulls the
//   history to the new update (weight 1/2) and restarts its count. That one test stands for every
//   change: lights, sky, movers, cuts.
const GI_W_ACC_UPDATES:u32=210u;
const GI_W_ACC_RAYS:u32=211u;
const GI_W_ACC_PULLS:u32=212u;
const GI_W_ACC_SUN_RAYS:u32=213u;
const GI_W_ACC_SUN_LIT:u32=214u;
const GI_ACC_LUMA:vec3<f32>=vec3<f32>(0.2126,0.7152,0.0722);

// One class of this frame's updates (0: cells without history, 1: visible cells due, 2: the rest
// due), appended while the frame's rays (GI_H_ACC_BUDGET, 0: no cap) last. The classes run as three
// passes in that order, so a cell without history is never left for a cell that has one.
fn gi_acc_select(kind:u32, id:vec3<u32>, groups:vec3<u32>) {
    let slots=gi_frame.offsets.x;
    let frame=gi_word(GI_H_FRAME);
    let pv=max(gi_word(GI_H_ACC_VISIBLE),1u);
    let po=max(gi_word(GI_H_ACC_OTHER),1u);
    let budget=gi_word(GI_H_ACC_BUDGET);
    let k=min(max(gi_word(GI_H_ACC_RAYS),1u),64u);
    for(var slot=id.x;slot<slots;slot+=groups.x*64u) {
        let hot=gi_hot(slot);
        if (gi_table[hot]&GI_VALID)==0u { continue; }
        let cold=gi_cold(slot);
        if (gi_table[cold+GI_C_STATE]&(GI_TRACED|GI_ORPHAN))!=GI_TRACED { continue; }
        let fresh=gi_table[hot+GI_HOT_ACC_COUNT]==0u;
        let visible=gi_table[cold+GI_C_SEEN]+1u>=frame;
        let phase=gi_mix(slot)+frame;
        var take=false;
        if kind==0u { take=fresh; }
        if kind==1u { take=!fresh && visible && phase%pv==0u; }
        if kind==2u { take=!fresh && !visible && phase%po==0u; }
        if !take { continue; }
        let rays=select(k,64u,fresh);
        if budget!=0u {
            // GI_W_ACC_RAYS counts the selected rays here; gi_accumulate adds nothing to it.
            let before=atomicAdd(&gi_work[GI_W_ACC_RAYS],rays);
            if before>=budget { continue; }
        }
        let at=atomicAdd(&gi_work[GI_W_INTEGRATES],1u);
        if at<slots { atomicStore(&gi_work[gi_frame.offsets.w+at],slot); }
    }
}
@compute @workgroup_size(64) fn gi_acc_select_fresh(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) { gi_acc_select(0u,id,groups); }
@compute @workgroup_size(64) fn gi_acc_select_visible(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) { gi_acc_select(1u,id,groups); }
@compute @workgroup_size(64) fn gi_acc_select_other(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) { gi_acc_select(2u,id,groups); }

// A light's incident irradiance at p facing it, before visibility and the cosine:
// program_api.wgsl light_sample_untinted's arithmetic (inverse square from 1 cm, the range
// window, the spot cone). Transparent panes do not tint it: the cache traces through them.
fn gi_light_incidence(l:u32, p:vec3<f32>, to:vec3<f32>, distance:f32) -> vec3<f32> {
    let light=lights[l];
    if light.kind.x==0u { return light.rgb_inner.xyz; }
    let inverse=1.0/max(distance,0.01);
    var a=inverse*inverse;
    if light.position_range.w>0.0 {
        let ratio=min(distance/light.position_range.w,1.0);
        let window=max(1.0-ratio*ratio*ratio*ratio,0.0);
        a*=window*window;
    }
    if light.kind.x==2u {
        let cone=clamp((dot(-to,light.travel_outer.xyz)-light.travel_outer.w)/max(light.rgb_inner.w-light.travel_outer.w,1e-4),0.0,1.0);
        a*=cone*cone;
    }
    return light.rgb_inner.xyz*a;
}
// An update ray's hit radiance with direct light evaluated at the hit (Lumen's hit lighting):
// the hit cell's parts (gi_cells.wgsl GiParts) give emission + albedo / pi * (its indirect
// irradiance about its normal + each light reaching the hit point, one visibility ray each).
// A cell's outgoing radiance took its lights from one visibility ray at its own query point,
// so a cell on a shadow's edge sent all or none of the sun to every cell whose rays met it:
// the hangar's mosaic (docs/design-gi-accumulate.md). While accumulating, a cell's sky SH holds
// all its indirect incident radiance (absolute), and no bounce words.
fn gi_acc_hit_lit(slot:u32, p:vec3<f32>, hn:vec3<f32>) -> vec3<f32> {
    let parts=gi_parts_unpack(gi_outgoing[gi_parts_at(slot)]);
    let nc=gi_unpack_normal(gi_table[gi_cold(slot)+GI_C_NORMAL]);
    var e=gi_cell_sky_irradiance(gi_hot(slot),nc);
    let start=p+hn*gi_origin_offset(p,0.0);
    for(var l=0u;l<lighting.count;l+=1u) {
        if !gi_light_affects(l,p,hn) { continue; }
        let to=gi_light_direction(l,p);
        atomicAdd(&gi_work[GI_W_ACC_SUN_RAYS],1u);
        if gi_visible_within(start,to.xyz,to.w) {
            atomicAdd(&gi_work[GI_W_ACC_SUN_LIT],1u);
            e+=gi_light_incidence(l,p,to.xyz,to.w)*dot(hn,to.xyz);
        }
    }
    return parts.emission+parts.albedo*e*(1.0/3.141592653589793);
}
// Ray `lane` of `rays` in an update of cell `slot` on cache frame `frame`, in the cell's frame
// (z along its normal): stratified over the hemisphere's solid angle (uniform in z), turned by a
// rotation drawn per update and jittered per ray, so consecutive updates trace different rays and
// many updates cover the hemisphere evenly. src/gi_cache.rs acc_local_direction is its twin, and
// the tests there hold both to that (accumulate_directions_*).
fn gi_acc_local(slot:u32, frame:u32, lane:u32, rays:u32) -> vec3<f32> {
    let seed=gi_mix(slot^(frame*0x9e3779b9u));
    let turn=f32(seed>>8u)*(6.283185307179586/16777216.0);
    let jitter=f32(gi_mix(seed^lane)>>8u)*(1.0/16777216.0);
    let z=1.0-(f32(lane)+jitter)/f32(rays);
    let r=sqrt(max(1.0-z*z,0.0));
    let phi=f32(lane)*2.399963229728653+turn;
    return vec3<f32>(r*cos(phi),r*sin(phi),z);
}
// ---- guided rays (product sampling of the cell's own last estimate, as Lumen's screen probes
// sample the previous frame's radiance) ----
// A small bright part of a dark hemisphere (sky through the hangar's roof strips) is met by a
// few uniform rays per hundred updates, and the slowly converging mean holds each cell's error
// still: a static mosaic. So a share of each update's rays is drawn in proportion to the cell's
// stored radiance times the cosine, over 64 bins of equal solid angle (8 bands uniform in z by 8
// sectors in phi, in the cell's fixed frame); the rest stay uniform, so no direction starves.
// Every ray is weighted by the balance heuristic over both strategies' fixed counts,
// 1 / (n_u / 2 pi + n_g p_g(d)), which keeps the estimate unbiased whatever the guide.
const GI_ACC_BINS:u32=64u;
var<workgroup> gi_wg_guide:array<f32,64>;
var<workgroup> gi_wg_guide_total:f32;
var<workgroup> gi_wg_guide_share:u32;
fn gi_acc_bin(local:vec3<f32>) -> u32 {
    let j=min(u32(max(1.0-local.z,0.0)*8.0),7u);
    var phi=atan2(local.y,local.x);
    if phi<0.0 { phi+=6.283185307179586; }
    let k=min(u32(phi*(8.0/6.283185307179586)),7u);
    return j*8u+k;
}
// A local direction inside bin b at the fractions u (band, sector).
fn gi_acc_bin_direction(b:u32, u:vec2<f32>) -> vec3<f32> {
    let z=1.0-(f32(b/8u)+u.x)/8.0;
    let r=sqrt(max(1.0-z*z,0.0));
    let phi=(f32(b%8u)+u.y)*(6.283185307179586/8.0);
    return vec3<f32>(r*cos(phi),r*sin(phi),z);
}
// The guided density at local direction `local` (per steradian), from the workgroup's bins.
fn gi_acc_guide_pdf(local:vec3<f32>) -> f32 {
    return gi_wg_guide[gi_acc_bin(local)]/(gi_wg_guide_total*(6.283185307179586/f32(GI_ACC_BINS)));
}
var<workgroup> gi_wg_acc:array<f32,27u*64u>;
var<workgroup> gi_wg_acc_count:u32;
@compute @workgroup_size(64) fn gi_accumulate(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if lane==0u { gi_wg_acc_count=min(atomicLoad(&gi_work[GI_W_INTEGRATES]),gi_frame.offsets.x); }
    let count=workgroupUniformLoad(&gi_wg_acc_count);
    let requests=(gi_frame.bounce.z&GI_RELIGHT_REQUESTS)!=0u;
    for(var at=group.x;at<count;at+=groups.x) {
        let slot=atomicLoad(&gi_work[gi_frame.offsets.w+at]);
        let hot=gi_hot(slot);
        let cold=gi_cold(slot);
        let state=gi_table[cold+GI_C_STATE];
        let updates=gi_table[hot+GI_HOT_ACC_COUNT];
        let rays=select(min(max(gi_word(GI_H_ACC_RAYS),1u),64u),64u,updates==0u);
        // The guide: lane b evaluates the cell's stored radiance times the cosine at bin b's
        // centre; lane 0 turns the bins into a running sum. A fresh cell has no estimate.
        let frame0=gi_basis(gi_unpack_normal(gi_table[cold+GI_C_NORMAL]));
        // The share is the header's, the same for every lane: loaded through workgroup memory so
        // the guide's barriers sit in uniform control flow, and skipped whole when it is 0.
        if lane==0u { gi_wg_guide_share=bitcast<u32>(f32(min(gi_word(GI_H_ACC_GUIDE),100u))*0.01); }
        let share=bitcast<f32>(workgroupUniformLoad(&gi_wg_guide_share));
        var guided=0u;
        var total=0.0;
        var pick=0u;
        if share>0.0 {
            var value=0.0;
            if updates!=0u {
                let centre=gi_acc_bin_direction(lane,vec2<f32>(0.5));
                gi_sh_basis(frame0*centre);
                var l=vec3<f32>(0.0);
                for(var i=0u;i<9u;i+=1u) { l+=gi_y[i]*vec3<f32>(gi_half(hot,3u*i),gi_half(hot,3u*i+1u),gi_half(hot,3u*i+2u)); }
                value=max(dot(l,GI_ACC_LUMA),0.0)*centre.z;
            }
            gi_wg_guide[lane]=value;
            workgroupBarrier();
            if lane==0u {
                var sum=0.0;
                for(var b=0u;b<GI_ACC_BINS;b+=1u) { sum+=gi_wg_guide[b]; gi_wg_guide[b]=sum; }
                gi_wg_guide_total=sum;
            }
            workgroupBarrier();
            total=gi_wg_guide_total;
            // Guided rays: the last n_g lanes; none without a usable estimate.
            guided=select(0u,u32(f32(rays)*share),total>0.0 && total<3.0e38);
            // The guided lanes pick a bin by inverting the running sum, before lane 0 turns it
            // back into per-bin values for the density.
            if lane>=rays-guided && lane<rays {
                let pseed=gi_mix(slot^(gi_word(GI_H_FRAME)*0x9e3779b9u)^0x68bc21ebu);
                let goal=f32(gi_mix(pseed^lane)>>8u)*(1.0/16777216.0)*total;
                var lo=0u; var hi=GI_ACC_BINS-1u;
                while lo<hi { let mid=(lo+hi)/2u; if gi_wg_guide[mid]<goal { lo=mid+1u; } else { hi=mid; } }
                pick=lo;
            }
            workgroupBarrier();
            if lane==0u {
                for(var b=GI_ACC_BINS-1u;b>0u;b-=1u) { gi_wg_guide[b]-=gi_wg_guide[b-1u]; }
            }
            workgroupBarrier();
        }
        let plain=rays-guided;
        for(var i=0u;i<9u;i+=1u) { gi_cell_sh[i]=vec3<f32>(0.0); }
        if lane<rays {
            let c=gi_cell(slot);
            let n=gi_unpack_normal(gi_table[cold+GI_C_NORMAL]);
            let q=gi_point(cold);
            let frame=gi_basis(n);
            let seed=gi_mix(slot^(gi_word(GI_H_FRAME)*0x9e3779b9u));
            // The cell's area: each ray from a fresh point of its surface, found by a probe along
            // -n from half an edge above a random point of its footprint, so the value converges on
            // the cell's area mean. From one query point a cell on fine relief (the hangar's
            // corrugated walls) kept that point's view: groove or ridge, a fixed per-cell mosaic.
            var start=q;
            if gi_word(GI_H_ACC_AREA)!=0u {
                let h1=gi_mix(seed^(lane*0x85ebca6bu+1u));
                let h2=gi_mix(h1^0xc2b2ae35u);
                let a=f32(h1>>8u)*(1.0/16777216.0)-0.5;
                let b=f32(h2>>8u)*(1.0/16777216.0)-0.5;
                let above=q+frame[0]*(a*c.edge)+frame[1]*(b*c.edge)+n*(0.5*c.edge);
                let probe=gi_closest(above,-n,0.0,c.edge);
                if probe.hit { start=above-n*probe.t; }
            }
            let origin=start+n*gi_origin_offset(start,c.edge);
            let secondary=(state&GI_SECONDARY)!=0u;
            let tertiary=(state&GI_TERTIARY)!=0u;
            let band=select(select(GI_BAND_SECONDARY,GI_BAND_TERTIARY,secondary),0u,!requests || tertiary);
            // A rotation and a jitter drawn per update: over its updates a cell meets every
            // direction. Guided lanes draw a point of their picked bin instead.
            var local=gi_acc_local(slot,gi_word(GI_H_FRAME),lane,max(plain,1u));
            if lane>=plain {
                let h1=gi_mix(seed^(lane*0x27d4eb2fu+7u));
                let h2=gi_mix(h1^0x165667b1u);
                local=gi_acc_bin_direction(pick,vec2<f32>(f32(h1>>8u),f32(h2>>8u))*(1.0/16777216.0));
            }
            let d=frame*local;
            gi_hit_spread=gi_header_f32(GI_H_HIT_SPREAD);
            let h=gi_closest(origin,d,0.0,1e20);
            var radiance=vec3<f32>(0.0);
            if !h.hit {
                radiance=gi_sky_absolute(gi_sky(d));
            } else {
                let o=gi_hit_radiance(origin,d,h,band,tertiary);
                if o.w>GI_HIT_NONE && gi_word(GI_H_ACC_HITS)!=0u {
                    radiance=o.rgb;
                    if gi_word(GI_H_ACC_DIRECT)!=0u && gi_hit_slot!=0xffffffffu {
                        let hn=select(-h.normal,h.normal,dot(h.normal,-d)>=0.0);
                        radiance=gi_acc_hit_lit(gi_hit_slot,origin+d*h.t,hn);
                    }
                }
                // Measurement: what the sun costs evaluated at the hit (one visibility ray per
                // directional light), counted only (RENDERLAB_GI_ACC_HITSUN).
                if gi_word(GI_H_ACC_HITSUN)!=0u {
                    let p=origin+d*h.t;
                    let hn=select(-h.normal,h.normal,dot(h.normal,-d)>=0.0);
                    let start=p+hn*gi_origin_offset(p,0.0);
                    for(var l=0u;l<lighting.count;l+=1u) {
                        if lights[l].kind.x!=0u || all(lights[l].rgb_inner.xyz==vec3<f32>(0.0)) { continue; }
                        let to=gi_light_direction(l,p);
                        if dot(to.xyz,hn)<=0.0 { continue; }
                        atomicAdd(&gi_work[GI_W_ACC_SUN_RAYS],1u);
                        if gi_visible_within(start,to.xyz,1e20) { atomicAdd(&gi_work[GI_W_ACC_SUN_LIT],1u); }
                    }
                }
            }
            // Measurement (RENDERLAB_GI_ACC_SHOW): green when the cell's ray origin sees the
            // camera's eye, red when it does not.
            if gi_word(GI_H_ACC_SHOW)!=0u {
                let eye=vec3<f32>(gi_header_f32(GI_H_EYE),gi_header_f32(GI_H_EYE+1u),gi_header_f32(GI_H_EYE+2u));
                let to=eye-origin;
                let distance=length(to);
                let seen=gi_visible_within(origin,to/max(distance,1e-6),distance);
                radiance=select(vec3<f32>(1000.0,0.0,0.0),vec3<f32>(0.0,1000.0,0.0),seen);
            }
            gi_sh_basis(d);
            // The balance heuristic over both strategies' counts (uniform alone: 2 pi / rays).
            var density=f32(plain)/6.283185307179586;
            if guided>0u { density+=f32(guided)*gi_acc_guide_pdf(local); }
            let w=1.0/density;
            for(var i=0u;i<9u;i+=1u) { gi_cell_sh[i]=radiance*(w*gi_y[i]); }
        }
        // The lanes' sums into lane 0 in a fixed tree order.
        for(var i=0u;i<9u;i+=1u) {
            gi_wg_acc[(i*3u)*64u+lane]=gi_cell_sh[i].x;
            gi_wg_acc[(i*3u+1u)*64u+lane]=gi_cell_sh[i].y;
            gi_wg_acc[(i*3u+2u)*64u+lane]=gi_cell_sh[i].z;
        }
        workgroupBarrier();
        for(var stride=32u;stride>0u;stride/=2u) {
            if lane<stride {
                for(var j=0u;j<27u;j+=1u) { gi_wg_acc[j*64u+lane]+=gi_wg_acc[j*64u+lane+stride]; }
            }
            workgroupBarrier();
        }
        if lane==0u {
            let scale=bitcast<f32>(gi_table[hot+1u]);
            var sample:array<vec3<f32>,9>;
            for(var i=0u;i<9u;i+=1u) {
                sample[i]=vec3<f32>(gi_wg_acc[(i*3u)*64u],gi_wg_acc[(i*3u+1u)*64u],gi_wg_acc[(i*3u+2u)*64u]);
            }
            let l=max(dot(sample[0],GI_ACC_LUMA),0.0);
            var fast=bitcast<f32>(gi_table[hot+GI_HOT_ACC_FAST]);
            var mean=bitcast<f32>(gi_table[hot+GI_HOT_ACC_MEAN]);
            var moment=bitcast<f32>(gi_table[hot+GI_HOT_ACC_MOMENT]);
            var n=updates;
            var alpha=1.0;
            if n==0u {
                fast=l; mean=l; moment=l*l; n=1u;
            } else {
                let history=f32(max(gi_word(GI_H_ACC_HISTORY),1u));
                alpha=1.0/min(f32(n)+1.0,history);
                fast=mix(fast,l,0.5);
                let sigmas=f32(gi_word(GI_H_ACC_SIGMA))*0.01;
                let spread=sigmas*sqrt(max(moment-mean*mean,0.0))+0.02*mean+1e-12;
                if sigmas>0.0 && abs(fast-mean)>spread {
                    alpha=max(alpha,0.5);
                    n=1u;
                    atomicAdd(&gi_work[GI_W_ACC_PULLS],1u);
                }
                mean=mix(mean,l,alpha);
                moment=mix(moment,l*l,alpha);
                n=min(n+1u,u32(history));
            }
            for(var i=0u;i<9u;i+=1u) {
                let old=vec3<f32>(gi_half(hot,3u*i),gi_half(hot,3u*i+1u),gi_half(hot,3u*i+2u))*scale;
                gi_cell_sh[i]=mix(old,sample[i],alpha);
            }
            gi_cell_sh[9]=vec3<f32>(0.0);
            gi_store_sh(slot);
            gi_table[hot+GI_HOT_BOUNCE]=0u;
            gi_table[hot+GI_HOT_ACC_FAST]=bitcast<u32>(fast);
            gi_table[hot+GI_HOT_ACC_MEAN]=bitcast<u32>(mean);
            gi_table[hot+GI_HOT_ACC_MOMENT]=bitcast<u32>(moment);
            gi_table[hot+GI_HOT_ACC_COUNT]=n;
            gi_queue_shade(slot);
            atomicAdd(&gi_work[GI_W_ACC_UPDATES],1u);
            if gi_word(GI_H_ACC_BUDGET)==0u { atomicAdd(&gi_work[GI_W_ACC_RAYS],rays); }
        }
        workgroupBarrier();
    }
}
