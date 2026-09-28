// @name Amazing Surf
// @de linear
// @bailout 100
// @param scale 1.7 1 3
// @param min_radius 0.5 0.05 1
// @param fold_x 1 0 2
// @param fold_y 1 0 2
fn amazing_surf(z: ptr<function, vec3<f32>>, dr: ptr<function, f32>, c: vec3<f32>, P: amazing_surf_Params) {
    var v = *z;
    // Box fold in x and y only; z is left free, giving the "surf" sheets.
    v.x = abs(v.x + P.fold_x) - abs(v.x - P.fold_x) - v.x;
    v.y = abs(v.y + P.fold_y) - abs(v.y - P.fold_y) - v.y;
    let m = P.scale / clamp(dot(v, v), P.min_radius * P.min_radius, 1.0);
    *z = v * m + c;
    *dr = (*dr) * abs(m) + 1.0;
}
