// Non-temporal material-footprint filtering. Geometric data stays independent of a
// BRDF; these optional helpers implement the standard program's GGX experiments.
struct NormalStatistics { frame:mat3x3<f32>, mean:vec2<f32>, covariance:vec3<f32> };

fn normal_frame(input:SurfaceInput) -> mat3x3<f32> {
    var n=unit_normal(input.normal);
    if dot(input.normal,input.normal)<=1e-12 {n=input.geometric;}
    if dot(n,n)<0.5 {n=vec3<f32>(0.0,0.0,1.0);}
    let projected=input.tangent-n*dot(n,input.tangent);
    var t=unit_normal(projected);
    if dot(projected,projected)<=1e-12 {
        let axis=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(n.x)>0.9);
        t=unit_normal(axis-n*dot(n,axis));
    }
    var b=input.handedness*cross(n,t);
    if input.material.double_sided!=0u && !input.source_front {n=-n;t=-t;b=-b;}
    return mat3x3<f32>(t,b,n);
}

// Covariance is formed AFTER the hardware filters raw moments. Clamp the tiny
// negative eigenvalue caused by half precision, never a positive variance.
fn normal_statistics(input:SurfaceInput) -> NormalStatistics {
    let frame=normal_frame(input);
    if input.material.normal_variance!=2u {
        return NormalStatistics(frame,vec2<f32>(0.0),vec3<f32>(0.0));
    }
    let a=sample_normal_layer_grad(input,input.uv,input.uv_dx,input.uv_dy,0);
    let b=sample_normal_layer_grad(input,input.uv,input.uv_dx,input.uv_dy,1);
    let scale=clamp(input.material.normal_scale,-1e10,1e10);
    let mean=a.xy;
    let diagonal=max(a.zw-mean*mean,vec2<f32>(0.0));
    let cross_cov=clamp(b.x-mean.x*mean.y,-sqrt(diagonal.x*diagonal.y),sqrt(diagonal.x*diagonal.y));
    return NormalStatistics(frame,mean*scale,vec3<f32>(diagonal.x,cross_cov,diagonal.y)*scale*scale);
}

fn geometric_covariance(input:SurfaceInput,frame:mat3x3<f32>) -> vec3<f32> {
    let n=unit_normal(input.normal);let length2=max(dot(input.normal,input.normal),1e-12);
    let dx=(input.normal_dx-n*dot(n,input.normal_dx))*inverseSqrt(length2);
    let dy=(input.normal_dy-n*dot(n,input.normal_dy))*inverseSqrt(length2);
    let a=vec2<f32>(dot(dx,frame[0]),dot(dx,frame[1]));
    let b=vec2<f32>(dot(dy,frame[0]),dot(dy,frame[1]));
    return vec3<f32>(dot(vec2<f32>(a.x,b.x),vec2<f32>(a.x,b.x)),a.x*a.y+b.x*b.y,dot(vec2<f32>(a.y,b.y),vec2<f32>(a.y,b.y)))/12.0;
}

fn anisotropic_ggx(n:vec3<f32>,t:vec3<f32>,b:vec3<f32>,v:vec3<f32>,l:vec3<f32>,base:vec3<f32>,metal:f32,rough:f32,covariance:vec3<f32>) -> vec3<f32> {
    let nl=max(dot(n,l),0.0);let nv=max(dot(n,v),1e-7);
    if dot(l+v,l+v)<1e-20 {return base*((1.0-metal)/PI)*nl;}
    let h=unit_normal(l+v);let nh=max(dot(n,h),1e-7);
    let alpha=max(rough*rough,1e-4);let a2=alpha*alpha;
    let a=vec3<f32>(a2+2.0*covariance.x,2.0*covariance.y,a2+2.0*covariance.z);
    let determinant=max(a.x*a.z-a.y*a.y,a2*a2);
    let q=vec2<f32>(dot(h,t),dot(h,b));
    let d=1.0/(PI*sqrt(determinant)*pow(nh*nh+(a.z*q.x*q.x-2.0*a.y*q.x*q.y+a.x*q.y*q.y)/determinant,2.0));
    let vt=vec2<f32>(dot(v,t),dot(v,b));let lt=vec2<f32>(dot(l,t),dot(l,b));
    let gv=sqrt(nv*nv+a.x*vt.x*vt.x+2.0*a.y*vt.x*vt.y+a.z*vt.y*vt.y);
    let gl=sqrt(nl*nl+a.x*lt.x*lt.x+2.0*a.y*lt.x*lt.y+a.z*lt.y*lt.y);
    let visibility=0.5/max(nl*gv+nv*gl,1e-20);
    let f0=vec3<f32>(0.04)*(1.0-metal)+base*metal;
    let fresnel=f0+(vec3<f32>(1.0)-f0)*pow(1.0-clamp(dot(v,h),0.0,1.0),5.0);
    return (base*((1.0-metal)/PI)+d*visibility*fresnel)*nl;
}

// Standard-normal Gauss-Hermite nodes and weights. Every summand retains the
// original GGX alpha: convolution adds no artificial wide-GGX tail.
fn normal_node(i:u32,order:u32) -> vec2<f32> {
    if order==3u {
        let nodes=array<f32,3>(-1.732050808,0.0,1.732050808);
        let weights=array<f32,3>(0.166666667,0.666666667,0.166666667);
        return vec2<f32>(nodes[i],weights[i]);
    }
    let nodes=array<f32,5>(-2.856970014,-1.35562618,0.0,1.35562618,2.856970014);
    let weights=array<f32,5>(0.0112574113,0.222075922,0.533333333,0.222075922,0.0112574113);
    return vec2<f32>(nodes[i],weights[i]);
}

fn normal_mixture(statistics:NormalStatistics,v:vec3<f32>,l:vec3<f32>,base:vec3<f32>,metal:f32,rough:f32,order:u32) -> vec3<f32> {
    let c=statistics.covariance;
    let l00=sqrt(max(c.x,0.0));let l10=c.y/max(l00,1e-20);let l11=sqrt(max(c.z-l10*l10,0.0));
    var sum=vec3<f32>(0.0);
    for(var y=0u;y<order;y+=1u) {for(var x=0u;x<order;x+=1u) {
        let a=normal_node(x,order);let b=normal_node(y,order);
        let slope=statistics.mean+vec2<f32>(l00*a.x,l10*a.x+l11*b.x);
        let n=unit_normal(statistics.frame*vec3<f32>(slope,1.0));
        sum+=brdf(n,v,l,base,metal,rough)*(a.y*b.y);
    }}
    return sum;
}

// Same single-frame distribution, integrated over actual geometric footprint
// positions instead of replacing the box normal spread by another GGX lobe.
fn footprint_mixture(input:SurfaceInput,l:vec3<f32>,base:vec3<f32>,metal:f32,rough:f32,order:u32) -> vec3<f32> {
    let nodes=array<f32,3>(-0.387298335,0.0,0.387298335);
    let weights=array<f32,3>(0.277777778,0.444444444,0.277777778);
    var sum=vec3<f32>(0.0);
    let statistics=normal_statistics(input);
    for(var y=0u;y<3u;y+=1u) {for(var x=0u;x<3u;x+=1u) {
        var sample=input;
        sample.world+=input.world_dx*nodes[x]+input.world_dy*nodes[y];
        sample.normal+=input.normal_dx*nodes[x]+input.normal_dy*nodes[y];
        let v=unit_normal(input.eye-sample.world);
        if input.material.normal_variance==2u {
            sum+=normal_mixture(NormalStatistics(normal_frame(sample),statistics.mean,statistics.covariance),v,l,base,metal,rough,order)*(weights[x]*weights[y]);
        } else {
            sum+=brdf(normal_frame(sample)[2],v,l,base,metal,rough)*(weights[x]*weights[y]);
        }
    }}
    return sum;
}

// Experiment selectors: 1 anisotropic GGX; 2/3 nine-/25-lobe Gaussian mixture;
// 4/5 the mixtures with box-footprint geometric integration; 6 adds low-variance gating.
// 7 integrates the measured spatial distribution; 8 is the rejected adaptive hybrid.
// Materials without moments retain their material normal; arbitrary user shading
// can consume NormalStatistics directly instead of adopting this standard BRDF.
fn filtered_brdf(input:SurfaceInput,l:vec3<f32>,base:vec3<f32>,metal:f32,rough:f32,method:u32) -> vec3<f32> {
    let stats=normal_statistics(input);
    let g=geometric_covariance(input,stats.frame);
    let n=material_normal(input);let v=unit_normal(input.eye-input.world);
    // Legacy maps have lost their distribution. Do not replace their mapped
    // normals by the geometric frame or infer a false Gaussian from no data.
    if input.material.normal_mapped!=0u && input.material.normal_variance!=2u {return brdf(n,v,l,base,metal,rough);}
    if method==0u {return brdf(n,v,l,base,metal,rough);}
    let variance=stats.covariance+g;
    if variance.x+variance.z<=1e-12 {return brdf(n,v,l,base,metal,rough);}
    if method==6u && variance.x+variance.z < rough*rough*rough*rough*0.05 {
        return brdf(n,v,l,base,metal,rough);
    }
    if method==7u {return spatial_brdf(input,l,base,metal,rough);}
    if method==8u && input.material.normal_variance==2u {
        let h=unit_normal(l+v);
        let slope=vec2<f32>(dot(h,stats.frame[0]),dot(h,stats.frame[1]))/max(dot(h,stats.frame[2]),1e-6);
        let delta=slope-stats.mean;let c=stats.covariance;
        let determinant=max(c.x*c.z-c.y*c.y,1e-20);
        let distance=(c.z*delta.x*delta.x-2.0*c.y*delta.x*delta.y+c.x*delta.y*delta.y)/determinant;
        // A narrow intrinsic lobe outside the Gaussian quadrature's support needs
        // the actual current footprint's tail, not an invented wide-GGX tail.
        if rough*rough*rough*rough<c.x+c.z && distance>9.0 {return spatial_brdf(input,l,base,metal,rough);}
    }
    if method==1u {
        let mean_normal=unit_normal(stats.frame*vec3<f32>(stats.mean,1.0));
        let t=unit_normal(stats.frame[0]-mean_normal*dot(mean_normal,stats.frame[0]));
        let b=sign(dot(cross(mean_normal,t),stats.frame[1]))*cross(mean_normal,t);
        return anisotropic_ggx(mean_normal,t,b,v,l,base,metal,rough,variance);
    }
    if method>=4u {return footprint_mixture(input,l,base,metal,rough,select(3u,5u,method>=5u));}
    return normal_mixture(NormalStatistics(stats.frame,stats.mean,variance),v,l,base,metal,rough,select(3u,5u,method==3u));
}

// Direct spatial quadrature of the material/BRDF within one current pixel. No
// history, colour statistics, visibility reuse or neighbours. Independent
// centred strata, rather than the film reference's randomized subframe sequence.
fn spatial_brdf(input:SurfaceInput,l:vec3<f32>,base:vec3<f32>,metal:f32,rough:f32)->vec3<f32> {
    var sum=vec3<f32>(0.0);
    for(var y=0u;y<16u;y+=1u) {for(var x=0u;x<16u;x+=1u) {
        let offset=(vec2<f32>(f32(x),f32(y))+vec2<f32>(0.5))/16.0-vec2<f32>(0.5);
        var sample=input;
        sample.world+=input.world_dx*offset.x+input.world_dy*offset.y;
        sample.normal+=input.normal_dx*offset.x+input.normal_dy*offset.y;
        let frame=normal_frame(sample);
        var n=frame[2];
        if input.material.normal_variance==2u {
            let uv=input.uv+input.uv_dx*offset.x+input.uv_dy*offset.y;
            let value=sample_normal_layer_grad(input,uv,input.uv_dx/16.0,input.uv_dy/16.0,1).yzw;
            let scale=max(abs(input.material.normal_scale),1.0);
            let mapped=vec3<f32>(value.xy*(input.material.normal_scale/scale),value.z/scale);
            n=unit_normal(frame*unit_normal(mapped));
        }
        sum+=brdf(n,unit_normal(input.eye-sample.world),l,base,metal,rough)*(1.0/256.0);
    }}
    return sum;
}

// Standard-program opt-in. Keep the material's intrinsic GGX lobe and integrate
// the current footprint; moment-only Gaussian approximations failed U1b's gate.
fn specular_brdf(input:SurfaceInput,l:vec3<f32>,base:vec3<f32>,metal:f32,rough:f32)->vec3<f32> {
    return filtered_brdf(input,l,base,metal,rough,7u);
}
