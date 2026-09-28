// Display transform: linear HDR -> exposure -> tone map -> sRGB encode -> dither.
// Writes gamma-encoded values into an Rgba8Unorm texture (what egui expects).

struct DisplayUniforms {
    exposure: f32,      // linear multiplier (2^EV)
    tone_map: u32,      // 0 = clamp, 1 = ACES fitted, 2 = AgX
    dither: u32,        // 0 = off, 1 = on
    frame: u32,
};

@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var<uniform> u: DisplayUniforms;

// Stephen Hill's fitted ACES (sRGB in, sRGB out).
fn aces_fitted(color: vec3<f32>) -> vec3<f32> {
    let aces_in = mat3x3<f32>(
        vec3<f32>(0.59719, 0.07600, 0.02840),
        vec3<f32>(0.35458, 0.90834, 0.13383),
        vec3<f32>(0.04823, 0.01566, 0.83777),
    );
    let aces_out = mat3x3<f32>(
        vec3<f32>(1.60475, -0.10208, -0.00327),
        vec3<f32>(-0.53108, 1.10813, -0.07276),
        vec3<f32>(-0.07367, -0.00605, 1.07602),
    );
    let v = aces_in * color;
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return aces_out * (a / b);
}

// Minimal AgX (Troy Sobotka's AgX; polynomial fit by Benjamin Wrensch).
// Linear sRGB in, linear sRGB out.
fn agx(color: vec3<f32>) -> vec3<f32> {
    let inset = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let outset = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var v = inset * max(color, vec3<f32>(1.0e-10));
    v = clamp(log2(v), vec3<f32>(min_ev), vec3<f32>(max_ev));
    v = (v - min_ev) / (max_ev - min_ev);
    let x2 = v * v;
    let x4 = x2 * x2;
    v = 15.5 * x4 * x2 - 40.14 * x4 * v + 31.96 * x4 - 6.868 * x2 * v + 0.4298 * x2 + 0.1191 * v - 0.00232;
    v = outset * v;
    return pow(max(v, vec3<f32>(0.0)), vec3<f32>(2.2));
}

fn srgb_encode(linear: vec3<f32>) -> vec3<f32> {
    let c = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let lower = c * 12.92;
    let higher = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(higher, lower, c <= vec3<f32>(0.0031308));
}

fn hash(p: vec2<u32>, frame: u32) -> f32 {
    var h = p.x * 1973u + p.y * 9277u + frame * 26699u;
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h) / 4294967295.0;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(in.position.xy);
    var color = textureLoad(hdr, pixel, 0).rgb * u.exposure;

    if u.tone_map == 1u {
        color = aces_fitted(color);
    } else if u.tone_map == 2u {
        color = agx(color);
    }
    var encoded = srgb_encode(color);

    if u.dither == 1u {
        // Triangular-distributed noise of +-1 LSB hides 8-bit banding in smooth gradients.
        let n = hash(pixel, u.frame) + hash(pixel + vec2<u32>(7919u, 104729u), u.frame) - 1.0;
        encoded += n / 255.0;
    }
    return vec4<f32>(encoded, 1.0);
}
