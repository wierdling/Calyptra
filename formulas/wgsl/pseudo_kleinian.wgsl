// @name Pseudo-Kleinian
// @de custom
// @bailout 1000
// @param box_x 0.92436 0.5 1.5
// @param box_y 0.90756 0.5 1.5
// @param box_z 0.92436 0.5 1.5
// @param size 1 0.5 1.5
// @param offset_x 0 -1 1
// @param offset_z 0 -1 1
// @param thickness 0.92784 0.5 1.2
//
// Knighty's pseudo-Kleinian limit set: box folds and sphere inversions that
// build an endless, cave-like lattice. Best explored from inside.
fn pseudo_kleinian(z: ptr<function, vec3<f32>>, dr: ptr<function, f32>, c: vec3<f32>, P: pseudo_kleinian_Params) {
    let box_size = vec3<f32>(P.box_x, P.box_y, P.box_z);
    var v = 2.0 * clamp(*z, -box_size, box_size) - *z;
    let k = max(P.size / max(dot(v, v), 1.0e-12), 1.0);
    v *= k;
    *dr = (*dr) * k;
    *z = v + vec3<f32>(P.offset_x, 0.0, P.offset_z);
}

// The limit set is thin sheets around the z axis, not a solid: its distance
// is estimated from the folded point directly.
fn pseudo_kleinian_de(z: vec4<f32>, dr: f32, P: pseudo_kleinian_Params) -> f32 {
    let p = z.xyz;
    let rxy = length(p.xy);
    return max(rxy - P.thickness, abs(rxy * p.z) / max(length(p), 1.0e-12)) / abs(dr);
}
