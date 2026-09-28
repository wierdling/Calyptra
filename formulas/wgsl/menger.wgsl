// @name Menger sponge
// @de box
// @bailout 1000
// @param scale 3 1.5 4
// @param offset_x 1 0 2
// @param offset_y 1 0 2
// @param offset_z 1 0 2
fn menger(z: ptr<function, vec3<f32>>, dr: ptr<function, f32>, c: vec3<f32>, P: menger_Params) {
    var v = abs(*z);
    // Sort components descending (folds across the diagonal planes).
    if v.x < v.y { v = v.yxz; }
    if v.x < v.z { v = v.zyx; }
    if v.y < v.z { v = v.xzy; }
    let o = vec3<f32>(P.offset_x, P.offset_y, P.offset_z) * (P.scale - 1.0);
    v = v * P.scale - o;
    if v.z < -0.5 * o.z {
        v.z += o.z;
    }
    *z = v;
    *dr = (*dr) * P.scale;
}
