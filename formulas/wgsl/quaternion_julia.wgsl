// @name Quaternion Julia
// @de logarithmic
// @state vec4
// @bailout 4
// @param cx -0.2 -1.5 1.5
// @param cy 0.6 -1.5 1.5
// @param cz 0.2 -1.5 1.5
// @param cw 0.2 -1.5 1.5
// @param slice 0 -1.5 1.5
// @init_w slice
//
// z <- z^2 + c over the quaternions. The fractal is 4D; `slice` picks the
// 3D cross-section shown (the starting 4th coordinate). It is a Julia set
// by nature: the constant is (cx, cy, cz, cw), not the sample position.
fn quaternion_julia(z: ptr<function, vec4<f32>>, dr: ptr<function, f32>, c: vec3<f32>, P: quaternion_julia_Params) {
    let q = *z;
    *dr = 2.0 * length(q) * (*dr);
    // (a + v)^2 = a^2 - |v|^2 + 2av, with a = x, v = (y, z, w).
    *z = vec4<f32>(q.x * q.x - dot(q.yzw, q.yzw), 2.0 * q.x * q.yzw) + vec4<f32>(P.cx, P.cy, P.cz, P.cw);
}
