
#include <metal_stdlib>
using namespace metal;
struct BufferEntry { device uint* data; ulong resource_id; ulong metadata; };
struct TextureEntry { ulong gpu_va; texture2d<float> texture; ulong metadata; };
struct SamplerEntry { sampler state; ulong resource_id; ulong metadata; };
struct TopLevel {
    uint root[8];
    device TextureEntry* textures;
    device BufferEntry* buffers;
    device TextureEntry* storage_images;
    device BufferEntry* accelerations;
    device SamplerEntry* samplers;
    device SamplerEntry* comparison_samplers;
};
