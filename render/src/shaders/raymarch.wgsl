// Distance-estimated raymarcher. Output is linear HDR.
//
// This is a template: the formula composer (formulas crate) appends
// `fn de(p: vec3<f32>) -> vec2<f32>` (distance, orbit trap), which reads
// `u.iterations`, `u.bailout`, `u.julia_c` and `u.slot_params`.

struct Uniforms {
    cam_pos: vec4<f32>,       // xyz = position
    cam_right: vec4<f32>,     // xyz = right * tan(fov/2) * aspect
    cam_up: vec4<f32>,        // xyz = up * tan(fov/2)
    cam_forward: vec4<f32>,   // xyz = forward, w = pixel angle (radians per pixel)
    light_dir: vec4<f32>,     // xyz = towards light, w = intensity
    julia_c: vec4<f32>,       // xyz = Julia constant, w > 0.5 = Julia mode
    resolution: vec2<f32>,
    bailout: f32,
    iterations: u32,
    max_steps: u32,
    detail: f32,
    step_factor: f32,
    max_distance: f32,
    ambient: f32,
    specular: f32,
    ao_strength: f32,
    fog_density: f32,
    palette_offset: f32,
    palette_frequency: f32,
    _pad0: f32,
    _pad1: f32,
    slot_params: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

// Tetrahedral normal: 4 DE evaluations instead of 6.
fn normal_at(p: vec3<f32>, eps: f32) -> vec3<f32> {
    let k = vec2<f32>(1.0, -1.0);
    return normalize(
        k.xyy * de(p + k.xyy * eps).x +
        k.yyx * de(p + k.yyx * eps).x +
        k.yxy * de(p + k.yxy * eps).x +
        k.xxx * de(p + k.xxx * eps).x
    );
}

fn ambient_occlusion(p: vec3<f32>, n: vec3<f32>, scale: f32) -> f32 {
    var occlusion = 0.0;
    var weight = 1.0;
    for (var i = 1; i <= 5; i++) {
        let h = scale * f32(i) * 0.6;
        occlusion += weight * (h - de(p + n * h).x);
        weight *= 0.5;
    }
    return clamp(1.0 - u.ao_strength * occlusion / scale, 0.0, 1.0);
}

// Inigo Quilez cosine palette; M3 replaces this with user gradients.
fn palette(t: f32) -> vec3<f32> {
    let a = vec3<f32>(0.55, 0.45, 0.45);
    let b = vec3<f32>(0.45, 0.40, 0.35);
    let c = vec3<f32>(1.0, 1.0, 1.0);
    let d = vec3<f32>(0.00, 0.15, 0.30);
    return a + b * cos(6.2831853 * (c * t + d));
}

fn background(rd: vec3<f32>) -> vec3<f32> {
    let h = rd.y * 0.5 + 0.5;
    return mix(vec3<f32>(0.02, 0.02, 0.035), vec3<f32>(0.10, 0.12, 0.18), h);
}

struct Hit {
    t: f32,
    steps: u32,
    hit: bool,
};

fn march(ro: vec3<f32>, rd: vec3<f32>) -> Hit {
    var t = 0.0;
    var result: Hit;
    result.hit = false;
    for (var i = 0u; i < u.max_steps; i++) {
        let d = de(ro + rd * t).x;
        // Stop once within the pixel footprint (scaled by `detail`).
        if d < t * u.cam_forward.w * u.detail {
            result.hit = true;
            result.steps = i;
            break;
        }
        t += d * u.step_factor;
        if t > u.max_distance {
            result.steps = i;
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
        return vec4<f32>(sky, 1.0);
    }

    let p = ro + rd * hit.t;
    let footprint = max(hit.t * u.cam_forward.w, 1.0e-6);
    let n = normal_at(p, footprint * 0.5);
    let l = u.light_dir.xyz;

    let trap = de(p).y;
    let albedo = palette(u.palette_offset + u.palette_frequency * 0.5 * log(max(trap, 1.0e-6)));

    let diffuse = max(dot(n, l), 0.0) * u.light_dir.w;
    let half_vec = normalize(l - rd);
    let spec = u.specular * pow(max(dot(n, half_vec), 0.0), 48.0) * u.light_dir.w;
    let sky_light = u.ambient * (0.5 + 0.5 * n.y);
    let ao = ambient_occlusion(p, n, max(footprint * 8.0, 0.002));

    var color = albedo * (diffuse + sky_light * ao) * mix(0.4, 1.0, ao) + vec3<f32>(spec);
    // Distance fog towards the background.
    let fog = 1.0 - exp(-u.fog_density * hit.t * hit.t);
    color = mix(color, sky, fog);
    return vec4<f32>(color, 1.0);
}

// Probe: distance estimate at the camera (drives fly speed) and hit distance
// of the center ray (orbit pivot). Written as [de, center_t]; center_t < 0 = miss.
@group(1) @binding(0) var<storage, read_write> probe_out: array<f32, 2>;

@compute @workgroup_size(1)
fn probe_main() {
    let ro = u.cam_pos.xyz;
    probe_out[0] = de(ro).x;
    let hit = march(ro, normalize(u.cam_forward.xyz));
    probe_out[1] = select(-1.0, hit.t, hit.hit);
}
