// @name Mandelbulb
// @de logarithmic
// @bailout 2
// @param power 8 1 16
fn mandelbulb(z: ptr<function, vec3<f32>>, dr: ptr<function, f32>, c: vec3<f32>, P: mandelbulb_Params) {
    let r = max(length(*z), 1.0e-12);
    // Y is the symmetry axis so the bulb sits upright in a Y-up world.
    let theta = acos(clamp((*z).y / r, -1.0, 1.0)) * P.power;
    let phi = atan2((*z).z, (*z).x) * P.power;
    *dr = pow(r, P.power - 1.0) * P.power * (*dr) + 1.0;
    *z = pow(r, P.power) * vec3<f32>(sin(theta) * cos(phi), cos(theta), sin(theta) * sin(phi)) + c;
}
