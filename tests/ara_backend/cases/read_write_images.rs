//! Read-write storage images on the port: a kernel that loads and stores the same
//! `texture_storage_2d_array<r32uint, read_write>` texel in one dispatch (what U-RTVIS's
//! reflection history binds at 54).
use super::*;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    kernel::Kernel,
    shader::{InputKey, Program},
    transfer,
};

const SIZE: [u32; 2] = [8, 4];
const LAYERS: u32 = 3;
const PROBE: &str = r#"
@group(0) @binding(0) var history:texture_storage_2d_array<r32uint,read_write>;
@compute @workgroup_size(4,4) fn probe(@builtin(global_invocation_id) id:vec3<u32>) {
    let p=vec2<i32>(id.xy);
    for(var layer=0;layer<3;layer+=1) {
        let old=textureLoad(history,p,layer).x;
        textureStore(history,p,layer,vec4<u32>(old*3u+id.x+id.y*16u+u32(layer)*256u+1u));
    }
}
"#;

/// Each texel holds a seed; the kernel replaces it with a function of what it read, so the
/// read back image shows both the load and the store worked, on every layer. The Vulkan
/// lowering is checked too: its SPIR-V passes spirv-val where the SDK has it.
#[test]
fn a_kernel_reads_and_writes_an_r32uint_image_array() -> Result<()> {
    let path = root("read-write-images");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let program = Program::new(Pass::HandleArrayProbe, PROBE)?;
    let kernel = Kernel::from_program(&mut gpu, &cache, &program)?;
    let export = path.join("spirv");
    for stage in renderlab::backend_ara::cache::emit_spirv_program(&program, &export)? {
        let Ok(sdk) = std::env::var("VULKAN_SDK") else {
            break;
        };
        let validator = std::path::Path::new(&sdk)
            .join("bin")
            .join(if cfg!(windows) {
                "spirv-val.exe"
            } else {
                "spirv-val"
            });
        if !validator.is_file() {
            break;
        }
        let output = std::process::Command::new(&validator)
            .args(["--target-env", "vulkan1.3"])
            .arg(export.join(stage.spirv))
            .output()?;
        ensure!(
            output.status.success(),
            "read-write storage image SPIR-V: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let image = Texture::array(&mut gpu, SIZE, LAYERS, Format::R32Uint, true, false)?;
    let texels = (SIZE[0] * SIZE[1] * LAYERS) as usize;
    let seed = |i: usize| (i as u32).wrapping_mul(2654435761) >> 20;
    let bytes: Vec<u8> = (0..texels).flat_map(|i| seed(i).to_le_bytes()).collect();
    let mut graph = Graph::default();
    let mut rec = gpu.begin(Queue::Graphics);
    let staging = transfer::upload(&mut gpu, &mut graph, &mut rec, image.image, &bytes)?;
    let packet = HeapBindings::for_program(
        &mut gpu,
        &program,
        &[(
            InputKey {
                group: 0,
                binding: 0,
            },
            Input::Image(&image),
        )],
    )?;
    packet.import(&mut graph);
    kernel.record(&mut graph, &mut rec, &packet, [SIZE[0], SIZE[1], 1])?;
    let read = transfer::readback(
        &mut gpu,
        &mut graph,
        &mut rec,
        image.image,
        bytes.len() as u64,
    )?;
    let done = gpu.submit(rec.finish(), &[])?;
    gpu.wait(done)?;
    let data = gpu.read(read)?;
    let mut wrong = 0;
    for layer in 0..LAYERS as usize {
        for y in 0..SIZE[1] as usize {
            for x in 0..SIZE[0] as usize {
                let i = (layer * SIZE[1] as usize + y) * SIZE[0] as usize + x;
                let got = u32::from_le_bytes(data[i * 4..i * 4 + 4].try_into()?);
                let want = seed(i) * 3 + x as u32 + y as u32 * 16 + layer as u32 * 256 + 1;
                wrong += usize::from(got != want);
            }
        }
    }
    println!("read-write r32uint image array: {wrong} of {texels} texels wrong");
    ensure!(
        wrong == 0,
        "{wrong} of {texels} texels did not read and write back"
    );
    gpu.retire(Resource::Buffer(staging))?;
    Ok(())
}
