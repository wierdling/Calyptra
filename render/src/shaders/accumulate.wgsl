// Adds one HDR sample tile into a float32 accumulation buffer.
// TILE must match `TILE` in still.rs.

const TILE: u32 = 256u;

@group(0) @binding(0) var sample_tile: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> accum: array<vec4<f32>>;

@compute @workgroup_size(8, 8)
fn accumulate(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= TILE || id.y >= TILE {
        return;
    }
    let i = id.y * TILE + id.x;
    accum[i] += textureLoad(sample_tile, vec2<i32>(id.xy), 0);
}
