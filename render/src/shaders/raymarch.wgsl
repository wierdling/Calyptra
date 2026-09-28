// Distance-estimated raymarcher and path tracer. Output is linear HDR.
//
// This is a template: the formula composer (formulas crate) appends
// `fn de(p) -> f32` and `fn de_trap(p) -> vec4<f32>`, which read
// `u.iterations`, `u.bailout`, `u.julia_c` and `u.slot_params`.
//
// Entry points: fs_preview (fast direct lighting), fs_pathtrace (Monte
// Carlo), fs_aux (albedo / normal / depth guides for the denoiser),
// probe_main (camera measurements read back by the CPU).

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
    resolution: vec2<f32>,      // full image size in pixels
    pixel_offset: vec2<f32>,    // added to the target pixel index (tile origin + sample position)
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
    sample_index: u32,          // random seed for the path tracer
    max_bounces: u32,
    aperture: f32,              // lens radius; 0 = pinhole
    focus_distance: f32,
    sun_cos_half_angle: f32,    // cosine of the sun disk's angular radius
    _pad0: f32,
    _pad1: f32,
    slot_params: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var gradient_tex: texture_2d<f32>;
@group(0) @binding(2) var gradient_sampler: sampler;

const PI: f32 = 3.14159265;

// ---------------------------------------------------------------- geometry

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

struct Hit {
    t: f32,
    hit: bool,
    // Smallest angular gap between the ray and the surface (radians); drives glow.
    min_angle: f32,
};

// `t_bias` is the distance already travelled before `ro` (for secondary
// rays): the hit threshold grows with total path length, like the pixel
// footprint does.
fn march_from(ro: vec3<f32>, rd: vec3<f32>, t_bias: f32) -> Hit {
    var t = 0.0;
    var result: Hit;
    result.hit = false;
    result.min_angle = 1.0e10;
    for (var i = 0u; i < u.max_steps; i++) {
        let d = de(ro + rd * t);
        // Stop once within the pixel footprint (scaled by `detail`).
        if d < (t + t_bias) * u.cam_forward.w * u.detail {
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

fn march(ro: vec3<f32>, rd: vec3<f32>) -> Hit {
    return march_from(ro, rd, 0.0);
}

// `pixel` is a continuous position in the full image, (0, 0) = top-left corner.
fn ray_dir(pixel: vec2<f32>) -> vec3<f32> {
    let uv = pixel / u.resolution;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    return normalize(u.cam_forward.xyz + ndc.x * u.cam_right.xyz + ndc.y * u.cam_up.xyz);
}

fn sample_pixel(position: vec4<f32>) -> vec2<f32> {
    return floor(position.xy) + u.pixel_offset;
}

// ---------------------------------------------------------------- shading

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

fn glow(hit: Hit) -> vec3<f32> {
    return u.glow_color.rgb * u.glow_color.w * exp(-hit.min_angle / max(u.glow_radius, 1.0e-5));
}

fn fog_amount(t: f32) -> f32 {
    return 1.0 - exp(-u.fog_density * t * t);
}

// ---------------------------------------------------------------- preview

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

@fragment
fn fs_preview(in: VsOut) -> @location(0) vec4<f32> {
    let ro = u.cam_pos.xyz;
    let rd = ray_dir(sample_pixel(in.position));
    let hit = march(ro, rd);
    let sky = background(rd);
    if !hit.hit {
        // Halo where rays grazed the fractal.
        return vec4<f32>(sky + glow(hit), 1.0);
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

    let color = albedo * (diffuse + ambient) * mix(0.5, 1.0, ao) + spec;
    return vec4<f32>(mix(color, sky, fog_amount(hit.t)), 1.0);
}

// ---------------------------------------------------------------- path tracer

var<private> rng_state: u32;

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rng_seed(pixel: vec2<u32>, sample: u32) {
    rng_state = pcg(pixel.x + pcg(pixel.y + pcg(sample)));
}

fn rand() -> f32 {
    rng_state = pcg(rng_state);
    return f32(rng_state >> 8u) / 16777216.0;
}

// Orthonormal basis around n (Duff et al. 2017).
fn basis(n: vec3<f32>) -> mat3x3<f32> {
    let s = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    let t = vec3<f32>(1.0 + s * n.x * n.x * a, s * b, -s * n.x);
    let bt = vec3<f32>(b, s + n.y * n.y * a, -n.y);
    return mat3x3<f32>(t, bt, n);
}

fn cosine_hemisphere(n: vec3<f32>) -> vec3<f32> {
    let r = sqrt(rand());
    let phi = 2.0 * PI * rand();
    let local = vec3<f32>(r * cos(phi), r * sin(phi), sqrt(max(0.0, 1.0 - r * r)));
    return basis(n) * local;
}

// Uniform direction within the cone around `axis` with the given cos(half angle).
fn sample_cone(axis: vec3<f32>, cos_max: f32) -> vec3<f32> {
    let cos_theta = mix(cos_max, 1.0, rand());
    let sin_theta = sqrt(max(0.0, 1.0 - cos_theta * cos_theta));
    let phi = 2.0 * PI * rand();
    return basis(axis) * vec3<f32>(sin_theta * cos(phi), sin_theta * sin(phi), cos_theta);
}

// Phong-lobe direction around `axis`: glossy reflections.
fn sample_phong(axis: vec3<f32>, exponent: f32) -> vec3<f32> {
    let cos_theta = pow(rand(), 1.0 / (exponent + 1.0));
    let sin_theta = sqrt(max(0.0, 1.0 - cos_theta * cos_theta));
    let phi = 2.0 * PI * rand();
    return basis(axis) * vec3<f32>(sin_theta * cos(phi), sin_theta * sin(phi), cos_theta);
}

fn occluded(ro: vec3<f32>, rd: vec3<f32>, t_bias: f32) -> bool {
    return march_from(ro, rd, t_bias).hit;
}

// Sky radiance: the background gradient, scaled like the preview's ambient
// so both modes have similar overall brightness.
fn sky_radiance(rd: vec3<f32>) -> vec3<f32> {
    return background(rd) * u.ambient * 4.0;
}

fn trace(ro_in: vec3<f32>, rd_in: vec3<f32>) -> vec3<f32> {
    var ro = ro_in;
    var rd = rd_in;
    var radiance = vec3<f32>(0.0);
    var throughput = vec3<f32>(1.0);
    var travelled = 0.0;
    var primary_t = -1.0;
    // Sun irradiance; π keeps diffuse brightness in line with the preview.
    let sun = u.light_color.rgb * u.light_dir.w * PI;
    // Fresnel-like reflectance of the glossy layer.
    let reflectance = clamp(u.specular * 0.08, 0.0, 0.9);

    for (var bounce = 0u; bounce <= u.max_bounces; bounce++) {
        let hit = march_from(ro, rd, travelled);
        if !hit.hit {
            // Camera rays see the backdrop as in the preview; bounced rays
            // are lit by it.
            let env = select(sky_radiance(rd), background(rd) + glow(hit), bounce == 0u);
            radiance += throughput * env;
            break;
        }
        let p = ro + rd * hit.t;
        travelled += hit.t;
        if bounce == 0u {
            primary_t = hit.t;
        }
        let footprint = max(travelled * u.cam_forward.w, 1.0e-6);
        let n = normal_at(p, footprint * 0.5);
        let albedo = surface_color(p, n);
        let origin = p + n * footprint * 2.0;

        // Direct sun light (next-event estimation) through a soft sun disk.
        let l = sample_cone(u.light_dir.xyz, u.sun_cos_half_angle);
        let n_dot_l = dot(n, l);
        if n_dot_l > 0.0 && !occluded(origin, l, travelled) {
            let h = normalize(l - rd);
            let spec = reflectance * (u.shininess + 8.0) / (8.0 * PI) * pow(max(dot(n, h), 0.0), u.shininess);
            radiance += throughput * sun * n_dot_l * (albedo * (1.0 - reflectance) / PI + spec);
        }

        // Continue the path: glossy reflection or diffuse bounce.
        if rand() < reflectance {
            rd = sample_phong(reflect(rd, n), u.shininess);
        } else {
            rd = cosine_hemisphere(n);
            throughput *= albedo;
        }
        if dot(rd, n) <= 0.0 {
            break;
        }
        ro = origin;

        // Russian roulette: stop dim paths early, unbiased.
        if bounce >= 2u {
            let keep = clamp(max(throughput.r, max(throughput.g, throughput.b)), 0.05, 1.0);
            if rand() > keep {
                break;
            }
            throughput /= keep;
        }
    }

    if primary_t >= 0.0 {
        radiance = mix(radiance, background(rd_in), fog_amount(primary_t));
    }
    return radiance;
}

@fragment
fn fs_pathtrace(in: VsOut) -> @location(0) vec4<f32> {
    let pixel = sample_pixel(in.position);
    rng_seed(vec2<u32>(in.position.xy) + vec2<u32>(u.pixel_offset), u.sample_index);
    var ro = u.cam_pos.xyz;
    var rd = ray_dir(pixel);

    // Thin lens: sample the aperture, aim at the focal plane.
    if u.aperture > 0.0 {
        let focus = ro + rd * (u.focus_distance / dot(rd, u.cam_forward.xyz));
        let r = u.aperture * sqrt(rand());
        let phi = 2.0 * PI * rand();
        ro += normalize(u.cam_right.xyz) * (r * cos(phi)) + normalize(u.cam_up.xyz) * (r * sin(phi));
        rd = normalize(focus - ro);
    }
    let color = trace(ro, rd);
    // Guard the accumulation against rare NaN / fireflies from degenerate normals.
    let safe = select(color, vec3<f32>(0.0), color != color);
    return vec4<f32>(min(safe, vec3<f32>(64.0)), 1.0);
}

// ---------------------------------------------------------------- denoiser guides

struct AuxOut {
    @location(0) albedo: vec4<f32>,
    // xyz = normal, w = hit distance (< 0 = background)
    @location(1) normal_depth: vec4<f32>,
};

@fragment
fn fs_aux(in: VsOut) -> AuxOut {
    let ro = u.cam_pos.xyz;
    let rd = ray_dir(sample_pixel(in.position));
    let hit = march(ro, rd);
    var out: AuxOut;
    if !hit.hit {
        out.albedo = vec4<f32>(1.0);
        out.normal_depth = vec4<f32>(0.0, 0.0, 0.0, -1.0);
        return out;
    }
    let p = ro + rd * hit.t;
    let n = normal_at(p, max(hit.t * u.cam_forward.w, 1.0e-6) * 0.5);
    out.albedo = vec4<f32>(surface_color(p, n), 1.0);
    out.normal_depth = vec4<f32>(n, hit.t);
    return out;
}

// ---------------------------------------------------------------- probe

// Read back by the CPU:
//   [0] distance estimate at the camera (drives fly speed)
//   [1] hit distance of the center ray (orbit pivot, focus); < 0 = miss
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
    let rd = ray_dir((vec2<f32>(id.xy) + 0.5) / f32(PROBE_GRID) * u.resolution);
    let hit = march(ro, rd);
    var value = PROBE_MISS;
    if hit.hit {
        let p = ro + rd * hit.t;
        let n = normal_at(p, max(hit.t * u.cam_forward.w, 1.0e-6) * 0.5);
        value = color_value(p, n);
    }
    probe_out[2u + id.y * PROBE_GRID + id.x] = value;
}
