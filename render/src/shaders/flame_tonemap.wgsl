// Flame tone mapping: turns histogram hit counts into brightness with a
// logarithm (flam3's key idea: detail across a huge dynamic range), smooths
// sparse areas with density estimation, then applies gamma and vibrancy.
//
// Density estimation spreads each pixel over a radius that shrinks as its
// density grows (flam3's rule), approximated with a pyramid so the cost
// doesn't depend on the radius:
// - `reduce` averages supersampled cells into output pixels, log-scales
//   them, and turns each pixel's radius into a continuous pyramid level.
// - `build_levels` sums each pixel into the two nearest levels (level
//   `l` is a grid of 2^l × 2^l pixel blocks; level -1 is the sharp image).
// - `fs_tonemap` adds the sharp image to every level upsampled with a
//   cubic B-spline, which spreads a block over about its own size.

/// Must match COLOR_SCALE in flame.wgsl.
const COLOR_SCALE: f32 = 255.0;
/// Blurred pyramid levels (block sizes 1, 2, 4, 8, 16 pixels).
const LEVELS: u32 = 5u;
/// A pyramid level with block size `s` matches the spread (variance) of
/// flam3's kernel with radius `KERNEL_TO_BLOCK * s`.
const KERNEL_TO_BLOCK: f32 = 1.83;

struct ToneParams {
    hist_size: vec2<u32>,
    supersample: u32,
    _pad0: u32,
    background: vec4<f32>,
    // Output pixels per plotted point: makes brightness independent of how
    // long the image has been refining.
    density_scale: f32,
    brightness: f32,
    gamma: f32,
    vibrancy: f32,
    // Density estimation (flam3's): a pixel with `d` hits per histogram
    // cell is spread over a radius of `clamp(max / d^curve, min, max)`
    // output pixels. `de_max_radius` 0 turns it off.
    de_max_radius: f32,
    de_min_radius: f32,
    de_curve: f32,
    _pad1: u32,
};

@group(0) @binding(0) var<uniform> tone: ToneParams;
@group(0) @binding(1) var<storage, read> counts: array<u32>;
// Per output pixel: (color * alpha, alpha), and its pyramid level.
@group(0) @binding(2) var<storage, read> pixels: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> pixel_levels: array<f32>;
// All blurred levels, one after another (see `level_offset`).
@group(0) @binding(4) var<storage, read> levels: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> pixels_out: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> pixel_levels_out: array<f32>;
@group(0) @binding(7) var<storage, read_write> levels_out: array<vec4<f32>>;

fn out_size() -> vec2<u32> {
    return tone.hist_size / tone.supersample;
}

fn level_size(level: u32) -> vec2<u32> {
    let block = 1u << level;
    return (out_size() + vec2<u32>(block - 1u)) / block;
}

fn level_offset(level: u32) -> u32 {
    var offset = 0u;
    for (var l = 0u; l < level; l++) {
        let size = level_size(l);
        offset += size.x * size.y;
    }
    return offset;
}

/// Share of a pixel at pyramid level `t` that goes to `level` (-1: sharp).
fn level_weight(t: f32, level: f32) -> f32 {
    return max(0.0, 1.0 - abs(t - level));
}

@compute @workgroup_size(8, 8)
fn reduce(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = out_size();
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let ss = tone.supersample;
    let base = id.xy * ss;
    var sum = vec4<f32>(0.0);
    for (var dy = 0u; dy < ss; dy++) {
        for (var dx = 0u; dx < ss; dx++) {
            let cell = ((base.y + dy) * tone.hist_size.x + base.x + dx) * 4u;
            sum += vec4<f32>(f32(counts[cell]), f32(counts[cell + 1u]), f32(counts[cell + 2u]), f32(counts[cell + 3u]));
        }
    }
    let i = id.y * size.x + id.x;
    let hits = sum.w;
    if hits <= 0.0 {
        pixels_out[i] = vec4<f32>(0.0);
        pixel_levels_out[i] = -1.0;
        return;
    }
    let color = sum.rgb / (hits * COLOR_SCALE);
    // Log density: ~0.1 in sparse areas, a few units in the densest.
    let alpha = 0.25 * tone.brightness * log(1.0 + hits * tone.density_scale);
    pixels_out[i] = vec4<f32>(color * alpha, alpha);
    var level = -1.0;
    if tone.de_max_radius > 0.0 {
        let density = hits / f32(ss * ss);
        let radius = clamp(
            tone.de_max_radius / pow(density, tone.de_curve),
            tone.de_min_radius,
            tone.de_max_radius,
        );
        level = clamp(log2(max(radius, 1.0e-3) / KERNEL_TO_BLOCK), -1.0, f32(LEVELS - 1u));
    }
    pixel_levels_out[i] = level;
}

// One thread per texel of every level (z = level): sums its block.
@compute @workgroup_size(8, 8)
fn build_levels(@builtin(global_invocation_id) id: vec3<u32>) {
    let level = id.z;
    let size = level_size(level);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let full = out_size();
    let block = 1u << level;
    let lo = id.xy * block;
    let hi = min(lo + vec2<u32>(block), full);
    var sum = vec4<f32>(0.0);
    for (var y = lo.y; y < hi.y; y++) {
        for (var x = lo.x; x < hi.x; x++) {
            let j = y * full.x + x;
            let w = level_weight(pixel_levels[j], f32(level));
            if w > 0.0 {
                sum += pixels[j] * w;
            }
        }
    }
    levels_out[level_offset(level) + id.y * size.x + id.x] = sum;
}

/// Cubic B-spline: smooth, support ±2, sums to 1 over integer shifts.
fn bspline(x: f32) -> f32 {
    let a = abs(x);
    if a < 1.0 {
        return (4.0 - 6.0 * a * a + 3.0 * a * a * a) / 6.0;
    }
    if a < 2.0 {
        let b = 2.0 - a;
        return b * b * b / 6.0;
    }
    return 0.0;
}

@fragment
fn fs_tonemap(in: VsOut) -> @location(0) vec4<f32> {
    let full = vec2<i32>(out_size());
    let p = vec2<i32>(in.position.xy);
    let i = p.y * full.x + p.x;
    var acc = pixels[i] * level_weight(pixel_levels[i], -1.0);
    if tone.de_max_radius > 0.0 {
        for (var level = 0u; level < LEVELS; level++) {
            let block = f32(1u << level);
            let size = vec2<i32>(level_size(level));
            let offset = level_offset(level);
            // This pixel's center in texel units (texel centers at k + 0.5).
            let u = in.position.xy / block - 0.5;
            let first = vec2<i32>(floor(u)) - vec2<i32>(1);
            for (var ty = 0; ty < 4; ty++) {
                let y = first.y + ty;
                if y < 0 || y >= size.y {
                    continue;
                }
                let wy = bspline(u.y - f32(y));
                for (var tx = 0; tx < 4; tx++) {
                    let x = first.x + tx;
                    if x < 0 || x >= size.x {
                        continue;
                    }
                    // Spread over block² pixels: keeps the block's energy.
                    let w = wy * bspline(u.x - f32(x)) / (block * block);
                    acc += levels[offset + u32(y * size.x + x)] * w;
                }
            }
        }
    }
    let alpha = acc.w;
    if alpha <= 0.0 {
        return vec4<f32>(tone.background.rgb, 1.0);
    }
    let color = acc.rgb / alpha;
    let alpha_gamma = pow(alpha, 1.0 / tone.gamma);
    // Vibrancy: gamma on density only (keeps saturation) vs per channel.
    let vivid = color * alpha_gamma;
    let flat = pow(color * alpha, vec3<f32>(1.0 / tone.gamma));
    let flame = mix(flat, vivid, tone.vibrancy);
    return vec4<f32>(mix(tone.background.rgb, flame, clamp(alpha_gamma, 0.0, 1.0)), 1.0);
}
