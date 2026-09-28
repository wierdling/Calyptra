use std::collections::BTreeSet;
use std::fmt::Write;

use scene::{DeMode, Fractal};

use crate::Library;

#[derive(Clone, Debug, PartialEq)]
pub enum ComposeError {
    NoSlots,
    TooManySlots,
    UnknownFormula(String),
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSlots => write!(f, "the fractal has no formula slots"),
            Self::TooManySlots => write!(f, "at most {} formula slots", Fractal::MAX_SLOTS),
            Self::UnknownFormula(id) => write!(f, "unknown formula {id:?}"),
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

    // Iteration `i` runs the slot whose repeat range contains `i % cycle`.
    let cycle: u32 = fractal.slots.iter().map(|s| s.repeat.max(1)).sum();
    let mut cases = String::new();
    let mut single_call = String::new();
    let mut start = 0;
    for (index, (slot, def)) in fractal.slots.iter().zip(&defs).enumerate() {
        let repeat = slot.repeat.max(1);
        let mut selectors: Vec<String> = (start..start + repeat).map(|n| format!("{n}u")).collect();
        if index == fractal.slots.len() - 1 {
            selectors.push("default".to_owned());
        }
        start += repeat;

        let args: Vec<String> = (0..def.params.len().max(1))
            .map(|j| {
                if def.params.is_empty() {
                    "0.0".to_owned()
                } else {
                    format!(
                        "u.slot_params[{}].{}",
                        2 * index + j / 4,
                        ["x", "y", "z", "w"][j % 4]
                    )
                }
            })
            .collect();
        single_call = format!(
            "{}(&z, &dr, c, {}_Params({}));",
            def.id,
            def.id,
            args.join(", ")
        );
        let _ = writeln!(
            cases,
            "            case {}: {{ {single_call} }}",
            selectors.join(", ")
        );
    }
    // A lone formula needs no dispatch.
    let step = if fractal.slots.len() == 1 {
        format!("        {single_call}\n")
    } else {
        format!("        switch i % {cycle}u {{\n{cases}        }}\n")
    };

    let de_mode = match fractal.de_mode {
        DeMode::Auto => defs[0].de_mode,
        mode => mode,
    };
    let distance = match de_mode {
        DeMode::Logarithmic | DeMode::Auto => "0.5 * log(r) * r / dr",
        DeMode::Linear => "r / abs(dr)",
        DeMode::Box => "(max(max(abs(z.x), abs(z.y)), abs(z.z)) - 1.0) / abs(dr)",
    };

    // The same iteration twice: `de` is the hot path used for marching,
    // normals, AO and shadows; `de_trap` runs once per pixel for coloring
    // and also records orbit statistics.
    let _ = write!(
        out,
        "// ---- distance estimator ----
fn de(p: vec3<f32>) -> f32 {{
    var z = p;
    var dr = 1.0;
    let c = select(p, u.julia_c.xyz, u.julia_c.w > 0.5);
    var r = length(z);
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
    var z = p;
    var dr = 1.0;
    let c = select(p, u.julia_c.xyz, u.julia_c.w > 0.5);
    var r = length(z);
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

#[cfg(test)]
mod tests {
    use super::*;
    use scene::FormulaSlot;

    #[test]
    fn single_formula_composes() {
        let library = Library::builtin().unwrap();
        let src = compose(&Fractal::default(), &library).unwrap();
        assert!(src.contains("struct mandelbulb_Params {\n    power: f32,"));
        assert!(
            src.contains(
                "        mandelbulb(&z, &dr, c, mandelbulb_Params(u.slot_params[0].x));\n"
            )
        );
        assert!(!src.contains("switch"));
        assert!(src.contains("0.5 * log(r) * r / dr"));
    }

    #[test]
    fn hybrid_repeats_and_shares_definitions() {
        let library = Library::builtin().unwrap();
        let mut box_slot = FormulaSlot::new("mandelbox", vec![]);
        box_slot.repeat = 2;
        let fractal = Fractal {
            slots: vec![
                box_slot,
                FormulaSlot::new("mandelbulb", vec![]),
                FormulaSlot::new("mandelbox", vec![]),
            ],
            ..Default::default()
        };
        let src = compose(&fractal, &library).unwrap();
        assert_eq!(src.matches("struct mandelbox_Params").count(), 1);
        assert!(src.contains("switch i % 4u"));
        assert!(src.contains("case 0u, 1u: { mandelbox("));
        assert!(src.contains("case 2u: { mandelbulb("));
        // Third slot's params start at element 4.
        assert!(src.contains(
            "case 3u, default: { mandelbox(&z, &dr, c, mandelbox_Params(u.slot_params[4].x"
        ));
        // Auto DE follows the first slot.
        assert!(src.contains("r / abs(dr)"));
    }

    #[test]
    fn unknown_formula_is_an_error() {
        let library = Library::builtin().unwrap();
        let fractal = Fractal {
            slots: vec![FormulaSlot::new("nope", vec![])],
            ..Default::default()
        };
        assert_eq!(
            compose(&fractal, &library),
            Err(ComposeError::UnknownFormula("nope".into()))
        );
    }
}
