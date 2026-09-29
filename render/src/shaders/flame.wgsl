// Fractal flames: the chaos game on the GPU, then log-density tone mapping.
//
// `chaos` runs many independent points in parallel. Each iteration picks a
// transform by weight, moves the point through it, blends the point's color
// toward the transform's, and splats the palette color into a histogram
// (atomic adds: red, green, blue, count per cell). Points persist across
// dispatches, so an image refines for as long as batches keep coming.
//
// Points are 3D. A 2D flame keeps z = 0 (identity z terms, 2D variations
// pass z through), and a camera without yaw, pitch or perspective looks
// straight down z, so 2D flames render exactly as before.
//
// Tone mapping is in flame_tonemap.wgsl.

const PI: f32 = 3.14159265;
const MAX_XFORMS: u32 = 12u;
/// Fixed-point scale for colors in the histogram.
const COLOR_SCALE: f32 = 255.0;

struct Variation {
    kind: u32,
    weight: f32,
    _pad0: f32,
    _pad1: f32,
    params: vec4<f32>,
};

// Rows of a 3×4 matrix: xyz = coefficients of (x, y, z), w = offset.
struct Xform {
    affine_x: vec4<f32>,    // a, b, xz, c
    affine_y: vec4<f32>,    // d, e, yz, f
    affine_z: vec4<f32>,    // zx, zy, zz, zc
    post_x: vec4<f32>,
    post_y: vec4<f32>,
    post_z: vec4<f32>,
    color: f32,
    color_speed: f32,
    cumulative_weight: f32, // running sum of normalized weights, for picking
    variation_count: u32,
    variations: array<Variation, 4>,
};

struct ChaosParams {
    hist_size: vec2<u32>,
    xform_count: u32,
    has_final: u32,
    camera: vec4<f32>,      // center x, center y, zoom, roll (radians)
    // Rows of the view rotation; w = perspective, depth of field, focus depth.
    view_x: vec4<f32>,
    view_y: vec4<f32>,
    view_z: vec4<f32>,
    iterations: u32,
    reset: u32,
    seed: u32,
    depth_fade: f32,
    preserve_z: u32,        // 1: 2D variations carry z (weighted); 0: they zero it
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var<uniform> chaos_params: ChaosParams;
// Transforms 0..11; index 12 is the final transform.
@group(0) @binding(1) var<storage, read> xforms: array<Xform, 13>;
@group(0) @binding(2) var<storage, read_write> points: array<vec4<f32>>;   // x, y, z, color
@group(0) @binding(3) var<storage, read_write> seeds: array<u32>;
@group(0) @binding(4) var<storage, read_write> histogram: array<atomic<u32>>;
@group(0) @binding(5) var gradient_tex: texture_2d<f32>;
@group(0) @binding(6) var gradient_sampler: sampler;

var<private> rng_state: u32;

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand() -> f32 {
    rng_state = pcg(rng_state);
    return f32(rng_state >> 8u) / 16777216.0;
}

// Variations that act on all three coordinates.
fn variation_3d(kind: u32, p: vec3<f32>, params: vec4<f32>) -> vec3<f32> {
    let r2 = max(dot(p, p), 1.0e-10);
    switch kind {
        case 29u: { return p / r2; }                                                // spherical3D
        case 30u: { return sin(p); }                                                // sinusoidal3D
        case 31u: {                                                                 // blur3D
            // Uniform in the unit ball.
            let u = rand() * 2.0 - 1.0;
            let a = rand() * 2.0 * PI;
            let s = sqrt(1.0 - u * u);
            return pow(rand(), 1.0 / 3.0) * vec3<f32>(s * cos(a), s * sin(a), u);
        }
        case 32u: {                                                                 // julia3D
            // JWildfire / Apophysis 3D hack, with an integer power.
            let power = select(round(params.x), 2.0, abs(round(params.x)) < 1.0);
            let abs_power = abs(power);
            let c_power = (1.0 / power - 1.0) * 0.5;
            let z = p.z / abs_power;
            let r2d = p.x * p.x + p.y * p.y;
            let r = pow(max(r2d + z * z, 1.0e-20), c_power);
            let r_xy = r * sqrt(r2d);
            let branch = floor(rand() * abs_power);
            let angle = (atan2(p.y, p.x) + 2.0 * PI * branch) / power;
            return vec3<f32>(r_xy * cos(angle), r_xy * sin(angle), r * z);
        }
        case 33u: {                                                                 // hemisphere
            let t = inverseSqrt(p.x * p.x + p.y * p.y + 1.0);
            return vec3<f32>(p.xy * t, t);
        }
        case 35u: { return vec3<f32>(0.0, 0.0, length(p.xy)); }                     // zcone
        case 36u: { return vec3<f32>(0.0, 0.0, 1.0); }                              // ztranslate
        case 37u: { return vec3<f32>(0.0, 0.0, p.z); }                              // zscale
        case 28u: { return p; }                                                     // linear3D
        default: { return p; }
    }
}

fn is_3d(kind: u32) -> bool {
    return (kind >= 28u && kind <= 33u) || (kind >= 35u && kind <= 37u);
}

fn variation(kind: u32, p: vec3<f32>, params: vec4<f32>) -> vec3<f32> {
    if is_3d(kind) {
        return variation_3d(kind, p, params);
    }
    // 2D variations shape x and y; z passes through only with preserve_z.
    let z = select(0.0, p.z, chaos_params.preserve_z == 1u);
    return vec3<f32>(variation_2d(kind, p.xy, params), z);
}

// Pre-variations adjust the transformed point before the others run.
fn pre_variation(kind: u32, p: vec3<f32>) -> vec3<f32> {
    switch kind {
        case 38u: {                                                                 // pre_blur
            // A Gaussian nudge in the plane.
            let r = rand() + rand() + rand() + rand() - 2.0;
            let a = rand() * 2.0 * PI;
            return vec3<f32>(r * cos(a), r * sin(a), 0.0);
        }
        default: { return vec3<f32>(0.0); }
    }
}

fn is_pre(kind: u32) -> bool {
    return kind == 38u;
}

fn variation_2d(kind: u32, p: vec2<f32>, params: vec4<f32>) -> vec2<f32> {
    let x = p.x;
    let y = p.y;
    let r2 = max(dot(p, p), 1.0e-10);
    let r = sqrt(r2);
    let theta = atan2(x, y);   // flam3's "atan": note the argument order
    let phi = atan2(y, x);
    switch kind {
        case 0u: { return p; }                                                      // linear
        case 1u: { return sin(p); }                                                 // sinusoidal
        case 2u: { return p / r2; }                                                 // spherical
        case 3u: {                                                                  // swirl
            let s = sin(r2);
            let c = cos(r2);
            return vec2<f32>(x * s - y * c, x * c + y * s);
        }
        case 4u: { return vec2<f32>((x - y) * (x + y), 2.0 * x * y) / r; }         // horseshoe
        case 5u: { return vec2<f32>(theta / PI, r - 1.0); }                         // polar
        case 6u: { return r * vec2<f32>(sin(theta + r), cos(theta - r)); }          // handkerchief
        case 7u: { return r * vec2<f32>(sin(theta * r), -cos(theta * r)); }         // heart
        case 8u: { return theta / PI * vec2<f32>(sin(PI * r), cos(PI * r)); }       // disc
        case 9u: { return vec2<f32>(cos(theta) + sin(r), sin(theta) - cos(r)) / r; } // spiral
        case 10u: { return vec2<f32>(sin(theta) / r, r * cos(theta)); }            // hyperbolic
        case 11u: { return vec2<f32>(sin(theta) * cos(r), cos(theta) * sin(r)); }  // diamond
        case 12u: {                                                                 // ex
            let p0 = sin(theta + r);
            let p1 = cos(theta - r);
            return r * vec2<f32>(p0 * p0 * p0 + p1 * p1 * p1, p0 * p0 * p0 - p1 * p1 * p1);
        }
        case 13u: {                                                                 // julia
            let a = phi * 0.5 + select(0.0, PI, rand() < 0.5);
            return sqrt(r) * vec2<f32>(cos(a), sin(a));
        }
        case 14u: {                                                                 // bent
            return vec2<f32>(select(x, 2.0 * x, x < 0.0), select(y, y * 0.5, y < 0.0));
        }
        case 15u: { return 2.0 / (r + 1.0) * vec2<f32>(y, x); }                    // fisheye
        case 16u: { return exp(x - 1.0) * vec2<f32>(cos(PI * y), sin(PI * y)); }   // exponential
        case 17u: { return pow(r, sin(theta)) * vec2<f32>(cos(theta), sin(theta)); } // power
        case 18u: {                                                                 // cosine
            return vec2<f32>(cos(PI * x) * cosh(y), -sin(PI * x) * sinh(y));
        }
        case 19u: { return 2.0 / (r + 1.0) * p; }                                   // eyefish
        case 20u: { return 4.0 / (r2 + 4.0) * p; }                                  // bubble
        case 21u: { return vec2<f32>(sin(x), y); }                                  // cylinder
        case 22u: { return vec2<f32>(sin(x) / cos(y), tan(y)); }                   // tangent
        case 23u: {                                                                 // cross
            return sqrt(1.0 / max((x * x - y * y) * (x * x - y * y), 1.0e-10)) * p;
        }
        case 24u: {                                                                 // blur
            let a = rand() * 2.0 * PI;
            return rand() * vec2<f32>(cos(a), sin(a));
        }
        case 25u: {                                                                 // julian
            let power = select(params.x, 1.0, abs(params.x) < 1.0e-3);
            let t = (phi + 2.0 * PI * floor(abs(power) * rand())) / power;
            let radius = pow(r2, params.y / (2.0 * power));
            return radius * vec2<f32>(cos(t), sin(t));
        }
        case 26u: {                                                                 // curl
            let t1 = 1.0 + params.x * x + params.y * (x * x - y * y);
            let t2 = params.x * y + 2.0 * params.y * x * y;
            return vec2<f32>(x * t1 + y * t2, y * t1 - x * t2) / max(t1 * t1 + t2 * t2, 1.0e-10);
        }
        case 34u: {                                                                 // separation
            let sx = params.x * params.x;
            let sy = params.y * params.y;
            return vec2<f32>(
                select(-(sqrt(x * x + sx) + x * params.z), sqrt(x * x + sx) - x * params.z, x > 0.0),
                select(-(sqrt(y * y + sy) + y * params.w), sqrt(y * y + sy) - y * params.w, y > 0.0),
            );
        }
        case 27u: {                                                                 // pdj
            return vec2<f32>(sin(params.x * y) - cos(params.y * x), sin(params.z * x) - cos(params.w * y));
        }
        default: { return p; }
    }
}

fn affine(x: vec4<f32>, y: vec4<f32>, z: vec4<f32>, p: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(x.xyz, p) + x.w, dot(y.xyz, p) + y.w, dot(z.xyz, p) + z.w);
}

fn apply_xform(xf: Xform, p: vec3<f32>) -> vec3<f32> {
    var t = affine(xf.affine_x, xf.affine_y, xf.affine_z, p);
    for (var i = 0u; i < xf.variation_count; i++) {
        let v = xf.variations[i];
        if is_pre(v.kind) {
            t += v.weight * pre_variation(v.kind, t);
        }
    }
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < xf.variation_count; i++) {
        let v = xf.variations[i];
        if !is_pre(v.kind) {
            sum += v.weight * variation(v.kind, t, v.params);
        }
    }
    return affine(xf.post_x, xf.post_y, xf.post_z, sum);
}

fn pick_xform() -> u32 {
    let r = rand();
    for (var i = 0u; i < chaos_params.xform_count; i++) {
        if r < xforms[i].cumulative_weight {
            return i;
        }
    }
    return chaos_params.xform_count - 1u;
}

fn valid(p: vec3<f32>) -> bool {
    // Rejects NaN (x != x) and runaway points.
    return all(p == p) && all(abs(p) < vec3<f32>(1.0e10));
}

fn plot(p: vec3<f32>, color: f32) {
    let cam = chaos_params.camera;
    // Into camera space: rotated by yaw and pitch. The view center is
    // applied after projection, in the image plane (as flam3 does).
    let view = vec3<f32>(
        dot(chaos_params.view_x.xyz, p),
        dot(chaos_params.view_y.xyz, p),
        dot(chaos_params.view_z.xyz, p),
    );
    // Perspective: points toward the viewer (larger z) appear larger.
    let depth_scale = 1.0 - chaos_params.view_x.w * view.z;
    if depth_scale < 1.0e-3 {
        return;
    }
    var flat = view.xy / depth_scale;
    // Depth of field: scatter points by their distance from the focal plane.
    let dof = chaos_params.view_y.w;
    if dof > 0.0 {
        let radius = dof * abs(view.z - chaos_params.view_z.w) * sqrt(rand());
        let a = rand() * 2.0 * PI;
        flat += radius * vec2<f32>(cos(a), sin(a));
    }
    flat -= cam.xy;
    // Roll within the image plane.
    let c = cos(cam.w);
    let s = sin(cam.w);
    let v = vec2<f32>(flat.x * c + flat.y * s, -flat.x * s + flat.y * c);
    let size = vec2<f32>(chaos_params.hist_size);
    let half_height = size.y * 0.5;
    let pixel = vec2<f32>(size.x * 0.5 + v.x * cam.z * half_height, half_height - v.y * cam.z * half_height);
    if pixel.x < 0.0 || pixel.y < 0.0 || pixel.x >= size.x || pixel.y >= size.y {
        return;
    }
    let cell = (u32(pixel.y) * chaos_params.hist_size.x + u32(pixel.x)) * 4u;
    // Depth cue: points behind the focal plane (smaller z) get darker.
    let fade = exp(-chaos_params.depth_fade * max(chaos_params.view_z.w - view.z, 0.0));
    let rgb = fade * textureSampleLevel(gradient_tex, gradient_sampler, vec2<f32>(color, 0.5), 0.0).rgb;
    let fixed = vec3<u32>(rgb * COLOR_SCALE + 0.5);
    atomicAdd(&histogram[cell], fixed.r);
    atomicAdd(&histogram[cell + 1u], fixed.g);
    atomicAdd(&histogram[cell + 2u], fixed.b);
    atomicAdd(&histogram[cell + 3u], 1u);
}

@compute @workgroup_size(64)
fn chaos(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= arrayLength(&points) {
        return;
    }
    var point = points[i];
    var skip = 0u;
    if chaos_params.reset == 1u {
        rng_state = pcg(i ^ pcg(chaos_params.seed));
        // z starts at 0: a 2D flame never leaves the plane.
        point = vec4<f32>(rand() * 2.0 - 1.0, rand() * 2.0 - 1.0, 0.0, rand());
        // Let the point settle onto the attractor before plotting.
        skip = 20u;
    } else {
        rng_state = seeds[i];
    }
    var p = point.xyz;
    var color = point.w;

    for (var n = 0u; n < chaos_params.iterations + skip; n++) {
        let xf = xforms[pick_xform()];
        p = apply_xform(xf, p);
        color = mix(color, xf.color, xf.color_speed);
        if !valid(p) {
            p = vec3<f32>(rand() * 2.0 - 1.0, rand() * 2.0 - 1.0, 0.0);
            color = rand();
            continue;
        }
        if n < skip {
            continue;
        }
        if chaos_params.has_final == 1u {
            let fin = xforms[MAX_XFORMS];
            let q = apply_xform(fin, p);
            if valid(q) {
                plot(q, mix(color, fin.color, fin.color_speed));
            }
        } else {
            plot(p, color);
        }
    }
    points[i] = vec4<f32>(p, color);
    seeds[i] = rng_state;
}
