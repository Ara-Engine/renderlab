//! Adapter conformance, separate from the count of original renderer passes.
use super::*;
use renderlab::backend_ara::{
    bindings::{HeapBindings, Input},
    buffer::StorageBuffer,
    kernel::Kernel,
    shader::InputKey,
    transfer::readback_buffer,
};
/// Words added to every probed buffer. On Metal none: the probe then reads past three
/// of them, and Naga's Metal output clamps such a read to the last element, which the
/// test holds it to. The SPIR-V carries no clamp (a read past the end is the device's
/// to define, as on the wgpu renderer with robust buffer access), so elsewhere every
/// read is kept in bounds and the test is about slots and lengths.
const SPARE: u32 = if cfg!(target_os = "macos") { 0 } else { 20 };

fn key(binding: u32) -> InputKey {
    InputKey { group: 0, binding }
}
#[test]
fn scattered_buffer_arrays_lengths_aliases_and_bounds_native_replay() -> Result<()> {
    let path = root("buffer-arrays");
    let mut gpu = GpuDevice::new_raster(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let kernel = Kernel::new(&mut gpu, &cache, Pass::BufferArrayProbe)?;
    let mut storage = Vec::new();
    let mut words = Vec::new();
    for i in 0..4u32 {
        // Lengths that are not all multiples of 16 bytes (32, 52, 76 and 88): a layer
        // or driver that rounds a buffer's bound range up reports a longer array, and
        // the renderer tells an index buffer with adjacency from one without by its
        // length modulo three.
        let mut data = vec![1000 + i, 0, 0, 0];
        data.extend((0..(i + 1) * 4 + [0, 1, 3, 2][i as usize] + SPARE).map(|j| i * 100 + j));
        storage.push(StorageBuffer::new(&mut gpu, bytemuck::cast_slice(&data))?);
        words.push(data);
        storage.push(StorageBuffer::new(&mut gpu, &[0; 16])?);
    }
    let counters = [
        StorageBuffer::new(&mut gpu, &[0; 16])?,
        StorageBuffer::new(&mut gpu, &[0; 16])?,
        StorageBuffer::new(&mut gpu, &[0; 16])?,
    ];
    let output = StorageBuffer::new(&mut gpu, &[0xff; 64])?;
    let mapping = [2usize, 0, 3, 2];
    let reads = mapping.map(|i| &storage[i * 2]);
    let writes = [&counters[1], &counters[0], &counters[2], &counters[1]];
    let values = [
        (key(0), Input::Buffers(&reads)),
        (key(1), Input::Buffer(&output)),
        (key(2), Input::Buffers(&writes)),
    ];
    let mut bad = values;
    bad[0].1 = Input::Buffers(&reads[..3]);
    ensure!(
        HeapBindings::new(&mut gpu, Pass::BufferArrayProbe, &bad).is_err(),
        "short binding array accepted"
    );
    let short = StorageBuffer::new(&mut gpu, &[0; 4])?;
    let short_reads = [&short, reads[1], reads[2], reads[3]];
    let mut bad_element = values;
    bad_element[0].1 = Input::Buffers(&short_reads);
    ensure!(
        HeapBindings::new(&mut gpu, Pass::BufferArrayProbe, &bad_element).is_err(),
        "short array element accepted"
    );
    let inputs = HeapBindings::new(&mut gpu, Pass::BufferArrayProbe, &values)?;
    let mut graph = Graph::default();
    for b in storage
        .iter()
        .chain(counters.iter())
        .chain(std::iter::once(&output))
    {
        graph.import(Resource::Buffer(b.buffer), Access::HostWrite);
    }
    inputs.import(&mut graph);
    let mut rec = gpu.begin(Queue::Graphics);
    kernel.record(&mut graph, &mut rec, &inputs, [4, 1, 1])?;
    let read = readback_buffer(&mut gpu, &mut graph, &mut rec, output.buffer, 0, output.len)?;
    let mut counts = Vec::new();
    for b in &counters {
        counts.push(readback_buffer(
            &mut gpu, &mut graph, &mut rec, b.buffer, 0, 4,
        )?);
    }
    let point = gpu.submit(rec.finish(), &[])?;
    gpu.wait(point)?;
    let data = gpu.read(read)?;
    let actual: Vec<_> = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| u32::from_le_bytes(*v))
        .collect();
    let mut expected = Vec::new();
    let mut sums = [0u32; 3];
    for (i, &m) in mapping.iter().enumerate() {
        let source = &words[m][4..];
        let value = source[(i * 7 + 2).min(source.len() - 1)];
        expected.extend([value, source.len() as u32, words[3][4 + i + 1], words[m][0]]);
        sums[[1, 0, 2, 1][i]] += value;
    }
    ensure!(
        actual == expected,
        "buffer array probe {actual:?} != {expected:?}"
    );
    for (read, expected) in counts.into_iter().zip(sums) {
        ensure!(
            u32::from_le_bytes(gpu.read(read)?[..4].try_into()?) == expected,
            "shared writable counter differs"
        );
        gpu.retire(Resource::Buffer(read))?;
    }
    println!(
        "buffer arrays: exact scattered slots, lengths, header offsets, restricted reads and shared atomic sums"
    );
    gpu.retire(Resource::Buffer(read))?;
    inputs.retire(&mut gpu, &mut graph)?;
    kernel.retire(&mut gpu)?;
    output.retire(&mut gpu)?;
    short.retire(&mut gpu)?;
    for b in storage.into_iter().chain(counters) {
        b.retire(&mut gpu)?;
    }
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
