// ---- T1 temporal film accumulation: the engine's store, after the program ----
// The temporal shading entry stores every pixel through film_store instead of writing
// primary_hdr directly (film_accumulation.rs FILM_ENTRY_SEAMS). History is the running mean of
// the program's exposed output and its sample weight, ping-ponged with the anchors, and kept
// only in tiles film_keep_tile flags. Weights never read colour, so every program converges
// to the footprint integral of its own output (docs/m6-temporal-primaries.md §6).
@group(0) @binding(26) var film_colour_in:texture_2d<f32>;
@group(0) @binding(27) var film_colour_out:texture_storage_2d<rgba32float,write>;
@group(0) @binding(28) var film_anchor_out:texture_storage_2d<rg32uint,write>;
// This frame's tile flags (the trace's film_keep_tile output).
@group(0) @binding(30) var film_tiles_now:texture_2d<u32>;
// The trace's current samples (film_init.wgsl film_sample_out): moving frames, covered tiles.
@group(0) @binding(31) var<storage,read> film_sample_in:array<u32>;
fn film_sample_at(pixel:vec2<i32>)->u32 {
    return film_sample_in[u32(pixel.x)+u32(pixel.y)*textureDimensions(primary_hit).x];
}

// The sequence index the trace used at a covered `pixel`. Under motion it may have traced a
// cold pixel again at the centre (film_closest).
fn film_traced_index(pixel:vec2<i32>)->u32 {
    if film_frame.moving==0u {return film_index(pixel);}
    return film_sample_at(pixel)&0xffu;
}

// What a background sample contributes: the clear colour as the f16 image stores it. History
// that has seen only background therefore equals it exactly, as the reference integrates
// stored samples, and needs no storage.
fn film_background()->vec3<f32> {
    return quantizeToF16(primary_frame.clear.rgb);
}

// Static window: the cumulative mean becomes an exponential mean of weight 1/256 beyond it.
const FILM_WINDOW:f32=256.0;

// History at a previous-frame tap whose tile flag is `kept`: stored, or implied.
fn film_history_at(tap:vec2<i32>,kept:bool)->vec4<f32> {
    if kept {return textureLoad(film_colour_in,tap,0);}
    return vec4<f32>(film_background(),f32(film_frame.implied));
}

// Reverse test of a tap with anchor `a`: its last sample point, carried by the camera
// (instances are static in T1b), projects within 1.5 px of `pixel`, and the current trace
// there hit its placement at a view distance within 2%. The point is rebuilt from the tap's
// own sample ray in the previous frame and its anchor distance.
fn film_reverse(a:vec2<u32>,tap:vec2<i32>,pixel:vec2<i32>,size:vec2<u32>)->bool {
    let placement=a.x&0x7fffffu;
    if placement==0u {return false;}
    let o=film_offset(a.x>>24u,vec2<u32>(tap),size.x);
    let ndc=fma((vec2<f32>(tap)+o)/vec2<f32>(size),vec2<f32>(2.0,-2.0),vec2<f32>(-1.0,1.0));
    let inverse=film_frame.previous_inverse;
    let xy=fma(inverse[0],vec4<f32>(ndc.x),inverse[1]*ndc.y);
    let near=xy+inverse[2]+inverse[3];
    let far=xy+inverse[3];
    let ray=far.xyz/far.w-near.xyz/near.w;
    let point=fma(ray,vec3<f32>(bitcast<f32>(a.y)/film_length(ray)),film_frame.previous_eye.xyz);
    let r=film_pixel(film_frame.current,vec4<f32>(point,1.0),vec2<f32>(size));
    let offset=r-vec2<f32>(pixel)-vec2<f32>(0.5);
    if fma(offset.x,offset.x,offset.y*offset.y)>2.25 {return false;}
    let at=vec2<i32>(floor(r));
    if any(at<vec2<i32>(0)) || any(at>=vec2<i32>(size)) {return false;}
    if textureLoad(primary_tiles,vec2<u32>(at)/PRIMARY_TILE,0).x==0u {return false;}
    if textureLoad(primary_hit,at,0).x!=placement {return false;}
    let d=film_length(point-lighting.eye);
    return abs(bitcast<f32>(film_sample_at(at)&0xffffff00u)-d)<=0.02*d;
}

// A previous-frame tap for surface `s`: valid when inside the image and its last sample passes
// the forward or reverse test, with its history when valid. One flag, anchor and colour load.
struct FilmTap { valid:bool, history:vec4<f32> };
fn film_tap(tap:vec2<i32>,s:FilmSurface,pixel:vec2<i32>,size:vec2<u32>)->FilmTap {
    if !film_inside(tap,size) {return FilmTap(false,vec4<f32>(0.0));}
    let kept=film_kept(tap);
    let a=film_anchor_at(tap,kept);
    if !film_forward(a,s) && !film_reverse(a,tap,pixel,size) {return FilmTap(false,vec4<f32>(0.0));}
    return FilmTap(true,film_history_at(tap,kept));
}

// Catmull-Rom weights of taps -1, 0, 1, 2 at fractional offset t.
fn film_catmull_rom(t:f32)->vec4<f32> {
    let t2=t*t;
    return vec4<f32>(t*fma(t,2.0-t,-1.0),fma(t2,fma(3.0,t,-5.0),2.0),
        t*fma(t,fma(-3.0,t,4.0),1.0),t2*(t-1.0))*0.5;
}

// Reprojected history of `pixel` for surface `s`: (mean, new weight n'), or zero weight when no
// inner tap validates (the pixel restarts). Catmull-Rom over 4x4 when all 16 taps validate,
// clamped at zero (non-negativity, not a colour clamp); otherwise bilinear over the valid
// inner taps, renormalised. The history weight is the bilinear-weighted tap count. Then
// n' = min(n + 1, W): W = 1 + v_max / s with s = sum f (1 - f), the resampling blur of the
// fetch's fractional offset f, and v_max = 1 px^2 (W = 256 at integer offsets).
fn film_reprojected(pixel:vec2<i32>,s:FilmSurface,size:vec2<u32>)->vec4<f32> {
    let q=film_fetch(vec2<u32>(pixel),s,vec2<f32>(size))-vec2<f32>(0.5);
    let base=vec2<i32>(floor(q));
    let f=q-floor(q);
    var weight=0.0;
    var colour=vec3<f32>(0.0);
    var n=0.0;
    var count=0u;
    // Histories of the 4x4 taps, row-major from (-1, -1).
    var taps=array<vec3<f32>,16>();
    for(var i=0;i<4;i+=1) {
        let t=film_tap(base+vec2<i32>(i&1,i>>1u),s,pixel,size);
        if t.valid {
            let wx=select(1.0-f.x,f.x,(i&1)!=0);let wy=select(1.0-f.y,f.y,(i>>1u)!=0);
            let w=wx*wy;
            weight=fma(wx,wy,weight);
            colour=fma(t.history.xyz,vec3<f32>(w),colour);
            n=fma(t.history.w,w,n);
            count+=1u;
            taps[5+(i&1)+4*(i>>1u)]=t.history.xyz;
        }
    }
    if count==0u || weight<=0.0 {return vec4<f32>(0.0);}
    colour/=weight;
    if count==4u {
        var outer=true;
        for(var i=0;i<16 && outer;i+=1) {
            let x=(i&3)-1;let y=(i>>2u)-1;
            if (x==0 || x==1) && (y==0 || y==1) {continue;}
            let t=film_tap(base+vec2<i32>(x,y),s,pixel,size);
            outer=t.valid;
            taps[i]=t.history.xyz;
        }
        if outer {
            let wx=film_catmull_rom(f.x);
            let wy=film_catmull_rom(f.y);
            var cr=vec3<f32>(0.0);
            for(var i=0;i<16;i+=1) {
                cr=fma(taps[i],vec3<f32>(wx[i&3]*wy[i>>2u]),cr);
            }
            colour=max(cr,vec3<f32>(0.0));
        }
    }
    let blur=fma(f.x,1.0-f.x,f.y*(1.0-f.y));
    var window=FILM_WINDOW;
    if blur>0.0 {window=clamp(1.0+1.0/blur,1.0,FILM_WINDOW);}
    return vec4<f32>(colour,min(n+1.0,window));
}

// The history this frame's sample on surface `s` joins at `pixel`: (mean, new weight n'), or
// zero weight when the pixel restarts (a reset, or no valid tap). Static frames read the
// pixel's own history; moving frames reproject it. Hit pixels fetch it before the program
// runs (FILM_ENTRY_SEAMS), so its loads can overlap the program's.
fn film_prior(pixel:vec2<i32>,s:FilmSurface)->vec4<f32> {
    if film_frame.reset!=0u || textureLoad(film_tiles_now,vec2<u32>(pixel)/PRIMARY_TILE,0).x==0u {
        return vec4<f32>(0.0);
    }
    if film_frame.moving!=0u {return film_reprojected(pixel,s,textureDimensions(primary_hit));}
    // A tile hit for the first time starts from its implied background history.
    let history=film_history_at(pixel,film_kept(pixel));
    return vec4<f32>(history.xyz,min(history.w+1.0,FILM_WINDOW));
}

// Accumulates one sample of the program's output at `pixel` (finite, nonnegative exposed
// HDR from rl_finite, or film_background) into `prior` (film_prior) and stores the film
// value. `k` is the sample's sequence index and `s` its surface.
fn film_write(pixel:vec2<i32>,sample:vec3<f32>,k:u32,s:FilmSurface,prior:vec4<f32>) {
    let tile=vec2<u32>(pixel)/PRIMARY_TILE;
    // The tile keeps no history: this sample and every one before it were background.
    if textureLoad(film_tiles_now,tile,0).x==0u {
        textureStore(primary_hdr,pixel,vec4<f32>(sample,1.0));
        return;
    }
    var mean=sample;
    var n=1.0;
    if prior.w>0.0 {
        n=prior.w;
        // m' = m + (x - m) / n', with the multiply-add pinned so repeats are bit-identical.
        mean=fma(sample-prior.xyz,vec3<f32>(1.0/n),prior.xyz);
    }
    textureStore(film_colour_out,pixel,vec4<f32>(mean,n));
    let anchor=(s.placement&0x7fffffu)|((s.side&1u)<<23u)|(k<<24u);
    textureStore(film_anchor_out,pixel,vec4<u32>(anchor,bitcast<u32>(film_distance(s)),0u,0u));
    textureStore(primary_hdr,pixel,vec4<f32>(mean,1.0));
}
fn film_store(pixel:vec2<i32>,sample:vec3<f32>,k:u32,s:FilmSurface) {
    film_write(pixel,sample,k,s,film_prior(pixel,s));
}

// Background pixels of the uncovered tiles and clear blocks: the background sample, with its
// surface computed only where the tile keeps history.
fn film_clear(p:vec2<u32>,size:vec2<u32>) {
    let pixel=vec2<i32>(p);
    if textureLoad(film_tiles_now,p/PRIMARY_TILE,0).x==0u {
        textureStore(primary_hdr,pixel,vec4<f32>(film_background(),1.0));
        return;
    }
    film_begin(film_index(pixel),p,size.x);
    film_store(pixel,film_background(),film_k,film_miss_surface(p,size));
}
// ---- end of the T1 store ----
