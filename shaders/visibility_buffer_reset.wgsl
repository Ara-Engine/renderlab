// End of frame, after resolve: keep each pass's counters for diagnostics (words 16..32),
// zero its working words and, with the pass-1 group, the reject header (count and
// pyramid dispatches), so the next frame starts without buffer clears.
@group(0) @binding(5) var<storage,read_write> words:array<u32>;
@group(0) @binding(6) var<storage,read_write> header:array<u32>;
@compute @workgroup_size(128) fn reset(@builtin(local_invocation_index) i:u32) {
    if i<16u {words[16u+i]=words[i];words[i]=0u;}
    if config.info.z==0u && i<100u {header[i]=0u;}
}
