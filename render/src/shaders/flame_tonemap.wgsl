// Flame tone mapping: turns histogram hit counts into brightness with a
// logarithm (flam3's key idea: detail across a huge dynamic range), applies
// gamma and vibrancy, and averages supersampled cells into output pixels.

/// Must match COLOR_SCALE in flame.wgsl.
const COLOR_SCALE: f32 = 255.0;

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
};

@group(0) @binding(0) var<uniform> tone: ToneParams;
@group(0) @binding(1) var<storage, read> counts: array<u32>;

@fragment
fn fs_tonemap(in: VsOut) -> @location(0) vec4<f32> {
    let ss = tone.supersample;
    let base = vec2<u32>(in.position.xy) * ss;
    var sum = vec4<f32>(0.0);
    for (var dy = 0u; dy < ss; dy++) {
        for (var dx = 0u; dx < ss; dx++) {
            let cell = ((base.y + dy) * tone.hist_size.x + base.x + dx) * 4u;
            sum += vec4<f32>(f32(counts[cell]), f32(counts[cell + 1u]), f32(counts[cell + 2u]), f32(counts[cell + 3u]));
        }
    }
    let hits = sum.w;
    if hits <= 0.0 {
        return vec4<f32>(tone.background.rgb, 1.0);
    }
    let color = sum.rgb / (hits * COLOR_SCALE);
    // Log density: ~0.1 in sparse areas, a few units in the densest.
    let alpha = 0.25 * tone.brightness * log(1.0 + hits * tone.density_scale);
    let alpha_gamma = pow(alpha, 1.0 / tone.gamma);
    // Vibrancy: gamma on density only (keeps saturation) vs per channel.
    let vivid = color * alpha_gamma;
    let flat = pow(color * alpha, vec3<f32>(1.0 / tone.gamma));
    let flame = mix(flat, vivid, tone.vibrancy);
    return vec4<f32>(mix(tone.background.rgb, flame, clamp(alpha_gamma, 0.0, 1.0)), 1.0);
}
