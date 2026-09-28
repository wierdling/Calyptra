// Distance-estimated raymarcher. Output is linear HDR.
//
// This is a template: the formula composer (formulas crate) appends
// `fn de(p) -> f32` and `fn de_trap(p) -> vec4<f32>`, which read
// `u.iterations`, `u.bailout`, `u.julia_c` and `u.slot_params`.

struct Uniforms {
    cam_pos: vec4<f32>,         // xyz = position
    cam_right: vec4<f32>,       // xyz = right * tan(fov/2) * aspect
    cam_up: vec4<f32>,          // xyz = up * tan(fov/2)
    cam_forward: vec4<f32>,     // xyz = forward, w = pixel angle (radians per pixel)
    light_dir: vec4<f32>,       // xyz = towards light, w = intensity
    light_color: vec4<f32>,     // rgb linear
    glow_color: vec4<f32>,      // rgb linear, w = intensity
    background_top: vec4<f32>,
    background_bottom: vec4<f32>,
    julia_c: vec4<f32>,         // xyz = Julia constant, w > 0.5 = Julia mode
    resolution: vec2<f32>,
    bailout: f32,
    iterations: u32,
    max_steps: u32,
    detail: f32,
    step_factor: f32,
    max_distance: f32,
    ambient: f32,
    specular: f32,
    shininess: f32,
    ao_strength: f32,
    shadow_sharpness: f32,      // 0 = shadows off
    fog_density: f32,
    glow_radius: f32,           // radians
    color_offset: f32,
    color_frequency: f32,
    color_source: u32,          // see scene::ColorSource
    color_wrap: u32,            // 0 repeat, 1 mirror, 2 clamp
    _pad: f32,
    slot_params: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var gradient_tex: texture_2d<f32>;
@group(0) @binding(2) var gradient_sampler: sampler;

// Tetrahedral normal: 4 DE evaluations instead of 6.
fn normal_at(p: vec3<f32>, eps: f32) -> vec3<f32> {
    let k = vec2<f32>(1.0, -1.0);
    return normalize(
        k.xyy * de(p + k.xyy * eps) +
        k.yyx * de(p + k.yyx * eps) +
        k.yxy * de(p + k.yxy * eps) +
        k.xxx * de(p + k.xxx * eps)
    );
}

fn ambient_occlusion(p: vec3<f32>, n: vec3<f32>, scale: f32) -> f32 {
    var occlusion = 0.0;
    var weight = 1.0;
    for (var i = 1; i <= 5; i++) {
        let h = scale * f32(i) * 0.6;
        occlusion += weight * (h - de(p + n * h));
        weight *= 0.5;
    }
    return clamp(1.0 - u.ao_strength * occlusion / scale, 0.0, 1.0);
}

// Penumbra estimate (Inigo Quilez): how close the shadow ray passes to the
// surface, relative to how far along it is.
fn soft_shadow(ro: vec3<f32>, rd: vec3<f32>, t_min: f32) -> f32 {
    var result = 1.0;
    var t = t_min;
    for (var i = 0; i < 96; i++) {
        let h = de(ro + rd * t);
        result = min(result, u.shadow_sharpness * h / t);
        if result < 0.002 || t > u.max_distance {
            break;
        }
        t += max(h * u.step_factor, t_min * 0.5);
    }
    return smoothstep(0.0, 1.0, clamp(result, 0.0, 1.0));
}

fn wrap_gradient(t: f32) -> f32 {
    switch u.color_wrap {
        case 1u: { return 1.0 - abs(fract(t * 0.5) * 2.0 - 1.0); }
        case 2u: { return clamp(t, 0.0, 1.0); }
        default: { return fract(t); }
    }
}

// The raw value that drives the gradient, before offset and frequency.
fn color_value(p: vec3<f32>, n: vec3<f32>) -> f32 {
    let traps = de_trap(p);
    switch u.color_source {
        case 1u: { return log(max(traps.y, 1.0e-6)); }
        case 2u: { return traps.z; }
        case 3u: { return p.y; }
        case 4u: { return 0.5 - 0.5 * n.y; }
        default: { return log(max(traps.x, 1.0e-6)); }
    }
}

fn surface_color(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let t = wrap_gradient(u.color_offset + u.color_frequency * color_value(p, n));
    // The lookup texture is sRGB; sampling returns linear color.
    return textureSampleLevel(gradient_tex, gradient_sampler, vec2<f32>(t, 0.5), 0.0).rgb;
}

fn background(rd: vec3<f32>) -> vec3<f32> {
    return mix(u.background_bottom.rgb, u.background_top.rgb, rd.y * 0.5 + 0.5);
}

struct Hit {
    t: f32,
    hit: bool,
    // Smallest angular gap between the ray and the surface (radians); drives glow.
    min_angle: f32,
};

fn march(ro: vec3<f32>, rd: vec3<f32>) -> Hit {
    var t = 0.0;
    var result: Hit;
    result.hit = false;
    result.min_angle = 1.0e10;
    for (var i = 0u; i < u.max_steps; i++) {
        let d = de(ro + rd * t);
        // Stop once within the pixel footprint (scaled by `detail`).
        if d < t * u.cam_forward.w * u.detail {
            result.hit = true;
            break;
        }
        if t > 0.0 {
            result.min_angle = min(result.min_angle, d / t);
        }
        t += d * u.step_factor;
        if t > u.max_distance {
            break;
        }
    }
    result.t = t;
    return result;
}

fn ray_dir(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    return normalize(u.cam_forward.xyz + ndc.x * u.cam_right.xyz + ndc.y * u.cam_up.xyz);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let ro = u.cam_pos.xyz;
    let rd = ray_dir(in.uv);
    let hit = march(ro, rd);
    let sky = background(rd);
    if !hit.hit {
        // Halo where rays grazed the fractal.
        let glow = exp(-hit.min_angle / max(u.glow_radius, 1.0e-5));
        return vec4<f32>(sky + u.glow_color.rgb * u.glow_color.w * glow, 1.0);
    }

    let p = ro + rd * hit.t;
    let footprint = max(hit.t * u.cam_forward.w, 1.0e-6);
    let n = normal_at(p, footprint * 0.5);
    let l = u.light_dir.xyz;
    let albedo = surface_color(p, n);

    // Offset along the normal so shadow and AO rays start outside the surface.
    let surface = p + n * footprint * 2.0;
    var shadow = 1.0;
    let n_dot_l = dot(n, l);
    if u.shadow_sharpness > 0.0 && n_dot_l > 0.0 {
        shadow = soft_shadow(surface, l, footprint * 4.0);
    }
    let ao = ambient_occlusion(p, n, max(footprint * 8.0, 0.002));

    let light = u.light_color.rgb * u.light_dir.w * shadow;
    let diffuse = max(n_dot_l, 0.0) * light;
    let half_vec = normalize(l - rd);
    // Normalized Blinn-Phong: sharper highlights get brighter, not dimmer.
    let spec_norm = (u.shininess + 8.0) / 25.13;
    let spec = u.specular * spec_norm * pow(max(dot(n, half_vec), 0.0), u.shininess) * light
        * step(0.0, n_dot_l);
    // Ambient comes from the environment: sky above, ground below.
    let ambient = u.ambient * background(n) * 4.0 * ao;

    var color = albedo * (diffuse + ambient) * mix(0.5, 1.0, ao) + spec;
    let fog = 1.0 - exp(-u.fog_density * hit.t * hit.t);
    color = mix(color, sky, fog);
    return vec4<f32>(color, 1.0);
}

// Probe, read back by the CPU:
//   [0] distance estimate at the camera (drives fly speed)
//   [1] hit distance of the center ray (orbit pivot); < 0 = miss
//   [2..] color values on a 16x16 grid of rays (gradient auto-fit); 1e30 = miss
const PROBE_GRID: u32 = 16u;
const PROBE_MISS: f32 = 1.0e30;
@group(1) @binding(0) var<storage, read_write> probe_out: array<f32, 258>;

@compute @workgroup_size(16, 16)
fn probe_main(@builtin(local_invocation_id) id: vec3<u32>) {
    let ro = u.cam_pos.xyz;
    if id.x == 0u && id.y == 0u {
        probe_out[0] = de(ro);
        let center = march(ro, normalize(u.cam_forward.xyz));
        probe_out[1] = select(-1.0, center.t, center.hit);
    }
    let rd = ray_dir((vec2<f32>(id.xy) + 0.5) / f32(PROBE_GRID));
    let hit = march(ro, rd);
    var value = PROBE_MISS;
    if hit.hit {
        let p = ro + rd * hit.t;
        let n = normal_at(p, max(hit.t * u.cam_forward.w, 1.0e-6) * 0.5);
        value = color_value(p, n);
    }
    probe_out[2u + id.y * PROBE_GRID + id.x] = value;
}
