// Progressive accumulation: adds one HDR sample into a float32 running sum
// and writes the running average for display.

struct Params {
    width: u32,
    height: u32,
    scale: f32,     // 1 / samples accumulated so far (including this one)
    reset: u32,     // 1 = this sample starts a new image
};

@group(0) @binding(0) var sample_tex: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> accum: array<vec4<f32>>;
@group(0) @binding(2) var resolved: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(8, 8)
fn accumulate(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let i = id.y * params.width + id.x;
    var sum = textureLoad(sample_tex, vec2<i32>(id.xy), 0);
    if params.reset == 0u {
        sum += accum[i];
    }
    accum[i] = sum;
    textureStore(resolved, vec2<i32>(id.xy), sum * params.scale);
}
