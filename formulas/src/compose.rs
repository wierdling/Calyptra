use std::collections::BTreeSet;
use std::fmt::Write;

use scene::{DeMode, Fractal};

use crate::{FormulaDef, Library};

#[derive(Clone, Debug, PartialEq)]
pub enum ComposeError {
    NoSlots,
    TooManySlots,
    UnknownFormula(String),
    /// Custom distance estimate selected, but no slot's formula defines one.
    NoCustomDe,
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSlots => write!(f, "the fractal has no formula slots"),
            Self::TooManySlots => write!(f, "at most {} formula slots", Fractal::MAX_SLOTS),
            Self::UnknownFormula(id) => write!(f, "unknown formula {id:?}"),
            Self::NoCustomDe => write!(
                f,
                "custom distance estimate selected, but no formula in the hybrid defines one"
            ),
        }
    }
}

impl std::error::Error for ComposeError {}

/// Generates WGSL defining, for `fractal`:
///
/// - `fn de(p: vec3<f32>) -> f32`: the distance estimate;
/// - `fn de_trap(p: vec3<f32>) -> vec4<f32>`: point trap, plane trap,
///   smoothed iteration count, distance estimate.
///
/// The output depends only on the fractal's *shape* (formulas, order,
/// repeats, DE mode) — parameter values, iterations, bailout and the Julia
/// constant are read at runtime from the host shader's uniforms:
///
/// - `u.iterations: u32`, `u.bailout: f32`
/// - `u.julia_c: vec4<f32>` (xyz = constant, w > 0.5 enables Julia mode)
/// - `u.slot_params: array<vec4<f32>, 16>`: slot `i`, parameter `j` is
///   component `j % 4` of element `2 * i + j / 4`.
pub fn compose(fractal: &Fractal, library: &Library) -> Result<String, ComposeError> {
    if fractal.slots.is_empty() {
        return Err(ComposeError::NoSlots);
    }
    if fractal.slots.len() > Fractal::MAX_SLOTS {
        return Err(ComposeError::TooManySlots);
    }
    let defs = fractal
        .slots
        .iter()
        .map(|slot| {
            library
                .get(&slot.formula)
                .ok_or_else(|| ComposeError::UnknownFormula(slot.formula.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = String::new();

    // Each distinct formula once: its params struct, then its source.
    let mut emitted = BTreeSet::new();
    for def in &defs {
        if !emitted.insert(def.id.as_str()) {
            continue;
        }
        let _ = writeln!(out, "// ---- formula: {} ----", def.id);
        let _ = writeln!(out, "struct {}_Params {{", def.id);
        if def.params.is_empty() {
            let _ = writeln!(out, "    _unused: f32,");
        }
        for param in &def.params {
            let _ = writeln!(out, "    {}: f32,", param.name);
        }
        let _ = writeln!(out, "}};\n\n{}\n", def.source.trim_end());
    }

    // `<id>_Params(...)` built from slot `index`'s uniforms.
    let params = |index: usize, def: &FormulaDef| {
        let args: Vec<String> = if def.params.is_empty() {
            vec!["0.0".to_owned()]
        } else {
            (0..def.params.len()).map(|j| param_ref(index, j)).collect()
        };
        format!("{}_Params({})", def.id, args.join(", "))
    };

    // The iterated point is always 4D; 3D formulas see (and update) xyz.
    let call = |index: usize, def: &FormulaDef| {
        if def.vec4_state {
            format!("{}(&z, &dr, c, {});", def.id, params(index, def))
        } else {
            format!(
                "var z3 = z.xyz; {}(&z3, &dr, c, {}); z = vec4<f32>(z3, z.w);",
                def.id,
                params(index, def)
            )
        }
    };

    // Iteration `i` runs the slot whose repeat range contains `i % cycle`.
    let cycle: u32 = fractal.slots.iter().map(|s| s.repeat.max(1)).sum();
    let mut cases = String::new();
    let mut start = 0;
    for (index, (slot, def)) in fractal.slots.iter().zip(&defs).enumerate() {
        let repeat = slot.repeat.max(1);
        let mut selectors: Vec<String> = (start..start + repeat).map(|n| format!("{n}u")).collect();
        if index == fractal.slots.len() - 1 {
            selectors.push("default".to_owned());
        }
        start += repeat;
        let _ = writeln!(
            cases,
            "            case {}: {{ {} }}",
            selectors.join(", "),
            call(index, def)
        );
    }
    // A lone formula needs no dispatch.
    let step = if fractal.slots.len() == 1 {
        format!("        {}\n", call(0, defs[0]))
    } else {
        format!("        switch i % {cycle}u {{\n{cases}        }}\n")
    };

    // Formulas that pick a 4D slice set the starting 4th coordinate.
    let mut init = String::new();
    for (index, def) in defs.iter().enumerate() {
        if let Some(j) = def.init_w {
            let _ = writeln!(init, "    z.w = {};", param_ref(index, j));
        }
    }

    let de_mode = match fractal.de_mode {
        DeMode::Auto => defs[0].de_mode,
        mode => mode,
    };
    let distance = match de_mode {
        DeMode::Logarithmic | DeMode::Auto => "0.5 * log(r) * r / dr".to_owned(),
        DeMode::Linear => "r / abs(dr)".to_owned(),
        DeMode::Box => "(max(max(abs(z.x), abs(z.y)), abs(z.z)) - 1.0) / abs(dr)".to_owned(),
        DeMode::Custom => {
            let (index, def) = defs
                .iter()
                .enumerate()
                .find(|(_, def)| def.custom_de)
                .ok_or(ComposeError::NoCustomDe)?;
            format!("{}_de(z, dr, {})", def.id, params(index, def))
        }
    };

    // The same iteration twice: `de` is the hot path used for marching,
    // normals, AO and shadows; `de_trap` runs once per pixel for coloring
    // and also records orbit statistics.
    let _ = write!(
        out,
        "// ---- distance estimator ----
fn de(p: vec3<f32>) -> f32 {{
    var z = vec4<f32>(p, 0.0);
    var dr = 1.0;
    let c = select(p, u.julia_c.xyz, u.julia_c.w > 0.5);
{init}    var r = length(z);
    for (var i = 0u; i < u.iterations; i++) {{
{step}        r = length(z);
        if r > u.bailout {{
            break;
        }}
    }}
    return {distance};
}}

// x = min |z|^2 (point trap), y = min distance to the axis planes,
// z = smoothed escape iteration, w = distance estimate.
fn de_trap(p: vec3<f32>) -> vec4<f32> {{
    var z = vec4<f32>(p, 0.0);
    var dr = 1.0;
    let c = select(p, u.julia_c.xyz, u.julia_c.w > 0.5);
{init}    var r = length(z);
    var point_trap = 1.0e10;
    var plane_trap = 1.0e10;
    var smooth_iter = f32(u.iterations);
    for (var i = 0u; i < u.iterations; i++) {{
{step}        point_trap = min(point_trap, dot(z, z));
        plane_trap = min(plane_trap, min(abs(z.x), min(abs(z.y), abs(z.z))));
        r = length(z);
        if r > u.bailout {{
            // Fractional part: how far past the bailout the orbit landed.
            let overshoot = log(r) / log(max(u.bailout, 1.0001));
            smooth_iter = f32(i) + 1.0 - clamp(log2(overshoot), 0.0, 1.0);
            break;
        }}
    }}
    return vec4<f32>(point_trap, plane_trap, smooth_iter, {distance});
}}
"
    );
    Ok(out)
}

/// WGSL expression for parameter `j` of slot `index`.
fn param_ref(index: usize, j: usize) -> String {
    format!(
        "u.slot_params[{}].{}",
        2 * index + j / 4,
        ["x", "y", "z", "w"][j % 4]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use scene::FormulaSlot;

    fn fractal(slots: Vec<FormulaSlot>) -> Fractal {
        Fractal {
            slots,
            ..Default::default()
        }
    }

    #[test]
    fn single_formula_composes() {
        let library = Library::builtin().unwrap();
        let src = compose(&Fractal::default(), &library).unwrap();
        assert!(src.contains(
            "struct mandelbulb_Params {
    power: f32,"
        ));
        assert!(src.contains(
            "var z3 = z.xyz; mandelbulb(&z3, &dr, c, mandelbulb_Params(u.slot_params[0].x));"
        ));
        assert!(!src.contains("switch"));
        assert!(src.contains("0.5 * log(r) * r / dr"));
    }

    #[test]
    fn hybrid_repeats_and_shares_definitions() {
        let library = Library::builtin().unwrap();
        let mut box_slot = FormulaSlot::new("mandelbox", vec![]);
        box_slot.repeat = 2;
        let src = compose(
            &fractal(vec![
                box_slot,
                FormulaSlot::new("mandelbulb", vec![]),
                FormulaSlot::new("mandelbox", vec![]),
            ]),
            &library,
        )
        .unwrap();
        assert_eq!(src.matches("struct mandelbox_Params").count(), 1);
        assert!(src.contains("switch i % 4u"));
        assert!(src.contains("case 0u, 1u: { var z3 = z.xyz; mandelbox("));
        assert!(src.contains("case 2u: { var z3 = z.xyz; mandelbulb("));
        // Third slot's params start at element 4.
        assert!(src.contains("case 3u, default: { var z3 = z.xyz; mandelbox(&z3, &dr, c, mandelbox_Params(u.slot_params[4].x"));
        // Auto DE follows the first slot.
        assert!(src.contains("r / abs(dr)"));
    }

    #[test]
    fn four_d_formulas_iterate_the_full_state_and_set_the_slice() {
        let library = Library::builtin().unwrap();
        let src = compose(
            &fractal(vec![FormulaSlot::new("quaternion_julia", vec![])]),
            &library,
        )
        .unwrap();
        assert!(src.contains("quaternion_julia(&z, &dr, c,"));
        // `slice` is the fifth parameter: element 1, x.
        assert!(src.contains("    z.w = u.slot_params[1].x;"));
    }

    #[test]
    fn custom_distance_estimate_uses_the_formula_function() {
        let library = Library::builtin().unwrap();
        let src = compose(
            &fractal(vec![FormulaSlot::new("pseudo_kleinian", vec![])]),
            &library,
        )
        .unwrap();
        assert!(src.contains("return pseudo_kleinian_de(z, dr, pseudo_kleinian_Params("));

        let bulb = Fractal {
            de_mode: DeMode::Custom,
            ..Default::default()
        };
        assert_eq!(compose(&bulb, &library), Err(ComposeError::NoCustomDe));
    }

    #[test]
    fn unknown_formula_is_an_error() {
        let library = Library::builtin().unwrap();
        assert_eq!(
            compose(&fractal(vec![FormulaSlot::new("nope", vec![])]), &library),
            Err(ComposeError::UnknownFormula("nope".into()))
        );
    }
}
