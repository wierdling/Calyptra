// @name KIFS octahedron
// @de linear
// @bailout 1000
// @param scale 2 1.2 3
// @param angle_x 0 -3.1416 3.1416
// @param angle_z 0 -3.1416 3.1416
// @param offset_x 1 0 2
// @param offset_y 0 0 2
// @param offset_z 0 0 2
fn kifs_octahedron_rotate(v: vec3<f32>, ax: f32, az: f32) -> vec3<f32> {
    let cx = cos(ax);
    let sx = sin(ax);
    let r1 = vec3<f32>(v.x, cx * v.y - sx * v.z, sx * v.y + cx * v.z);
    let cz = cos(az);
    let sz = sin(az);
    return vec3<f32>(cz * r1.x - sz * r1.y, sz * r1.x + cz * r1.y, r1.z);
}

fn kifs_octahedron(z: ptr<function, vec3<f32>>, dr: ptr<function, f32>, c: vec3<f32>, P: kifs_octahedron_Params) {
    // Octahedral symmetry: mirror into one fundamental domain.
    var v = abs(*z);
    if v.x < v.y { v = v.yxz; }
    if v.x < v.z { v = v.zyx; }
    if v.y < v.z { v = v.xzy; }
    v = kifs_octahedron_rotate(v, P.angle_x, P.angle_z);
    *z = v * P.scale - vec3<f32>(P.offset_x, P.offset_y, P.offset_z) * (P.scale - 1.0);
    *dr = (*dr) * P.scale;
}
