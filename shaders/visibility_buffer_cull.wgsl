@group(0) @binding(2) var<storage,read_write> visible:array<u32>;
struct Args {vertices:u32,count:atomic<u32>,first:u32,instance:u32};
@group(0) @binding(3) var<storage,read_write> args:Args;
@group(0) @binding(4) var<storage,read_write> rejected:array<u32>;
@group(0) @binding(5) var hzb:texture_2d<f32>;
@compute @workgroup_size(128) fn cull(@builtin(global_invocation_id) id:vec3<u32>) {
    let index=id.x+id.y*65535u*128u;
    if index>=config.info.x {return;}
    let second=config.info.z!=0u;
    if second && rejected[index]==0u {return;}
    if !second {rejected[index]=0u;}
    let b=cluster_box(clusters[index]);let lo=b[0];let hi=b[1];
    if lo.x>1.0 || hi.x< -1.0 || lo.y>1.0 || hi.y< -1.0 || hi.z<0.0 {return;}
    var hidden=false;
    if config.info.y!=0u && hi.z<1.0 {
        let size=vec2<f32>(vec2<u32>(config.info.w&65535u,config.info.w>>16u));
        let a=max(vec2<f32>(0.0),floor(vec2<f32>(lo.x+1.0,1.0-hi.y)*0.5*size)-vec2<f32>(1.0));
        let b=min(size-vec2<f32>(1.0),ceil(vec2<f32>(hi.x+1.0,1.0-lo.y)*0.5*size)+vec2<f32>(1.0));
        let level=min(u32(ceil(log2(max(max(b.x-a.x+1.0,b.y-a.y+1.0),1.0)))),textureNumLevels(hzb)-1u);
        let p=vec2<i32>(a)/i32(1u<<level);let q=vec2<i32>(b)/i32(1u<<level);
        let depth=min(min(textureLoad(hzb,p,i32(level)).x,textureLoad(hzb,q,i32(level)).x),min(textureLoad(hzb,vec2<i32>(p.x,q.y),i32(level)).x,textureLoad(hzb,vec2<i32>(q.x,p.y),i32(level)).x));
        hidden=hi.z<depth;
    }
    if hidden {if !second {rejected[index]=1u;}return;}
    visible[atomicAdd(&args.count,1u)]=index;
}
