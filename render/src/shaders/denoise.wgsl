// Edge-avoiding à-trous wavelet denoiser (Dammertz et al. 2010).
//
// Runs as a chain of full-screen passes:
//   mode 0: demodulate  — divide out surface albedo, so fractal color detail
//           is never blurred; only the (smooth) lighting is filtered.
//   mode 1: à-trous     — 5x5 B3-spline kernel with holes of size `step`,
//           weighted by normal, depth and color similarity.
//   mode 2: remodulate  — multiply the albedo back in.
// Background pixels (depth < 0) pass through untouched.

struct Params {
    step: i32,
    mode: u32,
    sigma_color: f32,
    pixel_angle: f32,   // radians per pixel: scales the depth tolerance
};

@group(0) @binding(0) var input_tex: texture_2d<f32>;
@group(0) @binding(1) var albedo_tex: texture_2d<f32>;
@group(0) @binding(2) var normal_depth_tex: texture_2d<f32>;
@group(0) @binding(3) var<uniform> params: Params;

fn safe_albedo(a: vec3<f32>) -> vec3<f32> {
    return max(a, vec3<f32>(0.02));
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Compress HDR so color distances are meaningful at any brightness.
fn compress(c: vec3<f32>) -> vec3<f32> {
    return c / (1.0 + luminance(c));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.position.xy);
    let color = textureLoad(input_tex, p, 0);
    let nd = textureLoad(normal_depth_tex, p, 0);
    if nd.w < 0.0 {
        return color;
    }
    let albedo = safe_albedo(textureLoad(albedo_tex, p, 0).rgb);
    if params.mode == 0u {
        return vec4<f32>(color.rgb / albedo, 1.0);
    }
    if params.mode == 2u {
        return vec4<f32>(color.rgb * albedo, 1.0);
    }

    let size = vec2<i32>(textureDimensions(input_tex));
    let kernel = array<f32, 5>(0.0625, 0.25, 0.375, 0.25, 0.0625);
    let center = compress(color.rgb);
    // Depth tolerance: a few pixel footprints per tap distance, so slanted
    // surfaces still blend while silhouettes do not.
    let depth_sigma = nd.w * params.pixel_angle * f32(params.step) * 4.0 + 1.0e-6;
    var sum = vec3<f32>(0.0);
    var weight_sum = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let q = clamp(p + vec2<i32>(dx, dy) * params.step, vec2<i32>(0), size - 1);
            let nd_q = textureLoad(normal_depth_tex, q, 0);
            if nd_q.w < 0.0 {
                continue;
            }
            let c_q = textureLoad(input_tex, q, 0).rgb;
            let w_normal = pow(max(dot(nd.xyz, nd_q.xyz), 0.0), 64.0);
            let w_depth = exp(-abs(nd.w - nd_q.w) / depth_sigma);
            let d = center - compress(c_q);
            let w_color = exp(-dot(d, d) / (params.sigma_color * params.sigma_color + 1.0e-8));
            let w = kernel[dx + 2] * kernel[dy + 2] * w_normal * w_depth * w_color;
            sum += c_q * w;
            weight_sum += w;
        }
    }
    return vec4<f32>(sum / max(weight_sum, 1.0e-8), 1.0);
}
