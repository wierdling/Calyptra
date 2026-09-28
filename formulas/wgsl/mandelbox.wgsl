// @name Mandelbox
// @de linear
// @bailout 100
// @param scale -1.5 -3 3
// @param min_radius 0.5 0 1.5
// @param fixed_radius 1 0.1 2
// @param fold_limit 1 0.1 2
fn mandelbox(z: ptr<function, vec3<f32>>, dr: ptr<function, f32>, c: vec3<f32>, P: mandelbox_Params) {
    // Box fold.
    var v = clamp(*z, vec3<f32>(-P.fold_limit), vec3<f32>(P.fold_limit)) * 2.0 - *z;
    // Sphere fold.
    let r2 = dot(v, v);
    let min_r2 = max(P.min_radius * P.min_radius, 1.0e-12);
    let fixed_r2 = P.fixed_radius * P.fixed_radius;
    var k = 1.0;
    if r2 < min_r2 {
        k = fixed_r2 / min_r2;
    } else if r2 < fixed_r2 {
        k = fixed_r2 / r2;
    }
    *z = v * k * P.scale + c;
    *dr = (*dr) * k * abs(P.scale) + 1.0;
}
