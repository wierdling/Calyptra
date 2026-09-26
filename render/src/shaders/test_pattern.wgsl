// M0 test pattern: raymarched sphere over a cosine-palette background.
// Output is linear HDR (values above 1.0 are expected in highlights).

struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

// Inigo Quilez cosine palette.
fn palette(t: f32) -> vec3<f32> {
    let a = vec3<f32>(0.5, 0.5, 0.5);
    let b = vec3<f32>(0.5, 0.5, 0.5);
    let c = vec3<f32>(1.0, 1.0, 1.0);
    let d = vec3<f32>(0.00, 0.33, 0.67);
    return a + b * cos(6.2831853 * (c * t + d));
}

fn scene_sdf(p: vec3<f32>) -> f32 {
    let wobble = 0.08 * sin(4.0 * p.x + u.time) * sin(4.0 * p.y + 1.3 * u.time) * sin(4.0 * p.z);
    return length(p) - 1.0 + wobble;
}

fn normal_at(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(0.001, 0.0);
    return normalize(vec3<f32>(
        scene_sdf(p + e.xyy) - scene_sdf(p - e.xyy),
        scene_sdf(p + e.yxy) - scene_sdf(p - e.yxy),
        scene_sdf(p + e.yyx) - scene_sdf(p - e.yyx),
    ));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let aspect = u.resolution.x / u.resolution.y;
    let ndc = vec2<f32>((in.uv.x * 2.0 - 1.0) * aspect, 1.0 - in.uv.y * 2.0);

    // Orbiting camera.
    let angle = 0.3 * u.time;
    let ro = vec3<f32>(3.0 * sin(angle), 0.8, 3.0 * cos(angle));
    let forward = normalize(-ro);
    let right = normalize(cross(forward, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(right, forward);
    let rd = normalize(forward * 1.8 + right * ndc.x + up * ndc.y);

    // Background: slow palette sweep, dimmed.
    var color = 0.15 * palette(0.25 * in.uv.y + 0.05 * u.time);

    var t = 0.0;
    for (var i = 0; i < 128; i++) {
        let d = scene_sdf(ro + rd * t);
        if d < 0.0005 * t {
            let p = ro + rd * t;
            let n = normal_at(p);
            let light_dir = normalize(vec3<f32>(0.6, 0.8, 0.4));
            let diffuse = max(dot(n, light_dir), 0.0);
            let half_vec = normalize(light_dir - rd);
            let specular = pow(max(dot(n, half_vec), 0.0), 64.0);
            let albedo = palette(0.35 * p.y + 0.1 * u.time);
            // Specular deliberately exceeds 1.0 to exercise tone mapping.
            color = albedo * (0.08 + diffuse) + vec3<f32>(4.0) * specular;
            break;
        }
        t += d;
        if t > 20.0 {
            break;
        }
    }
    return vec4<f32>(color, 1.0);
}
