//! Random fractals and mutation: raw candidates. Whether a candidate is
//! worth showing (not empty, not a featureless blob) is judged by
//! rendering it; see `render::random`.

use scene::{DeMode, FormulaSlot, Fractal};

use crate::{FormulaDef, Library, Origin};

/// Formulas that need special camera placement (an endless lattice seen
/// from inside) and so make poor random picks.
const EXCLUDED: &[&str] = &["pseudo_kleinian"];

/// A random single formula or 2–3 formula hybrid from the built-ins.
pub fn random_fractal(library: &Library, seed: u64) -> Fractal {
    let mut rng = Rng::new(seed);
    let candidates: Vec<&FormulaDef> = library
        .formulas()
        .filter(|d| d.origin == Origin::BuiltIn && !EXCLUDED.contains(&d.id.as_str()))
        .collect();
    assert!(!candidates.is_empty(), "no built-in formulas");

    let slot_count = if rng.unit() < 0.5 {
        1
    } else {
        2 + rng.below(2) as usize
    };
    let mut slots = Vec::with_capacity(slot_count);
    let mut defs = Vec::with_capacity(slot_count);
    for _ in 0..slot_count {
        let def = candidates[rng.below(candidates.len() as u32) as usize];
        let params = def
            .params
            .iter()
            .map(|p| {
                // Mostly near the default (where formulas are known to look
                // good), sometimes anywhere in range.
                let spread = if rng.unit() < 0.8 { 0.25 } else { 1.0 };
                let span = (p.max - p.min) * spread;
                (p.default + rng.range(-0.5, 0.5) * span).clamp(p.min, p.max)
            })
            .collect();
        let mut slot = FormulaSlot::new(&def.id, params);
        if slot_count > 1 {
            slot.repeat = 1 + rng.below(3);
        }
        slots.push(slot);
        defs.push(def);
    }

    let first = defs[0];
    let has_power_formula = defs.iter().any(|d| d.de_mode == DeMode::Logarithmic);
    let has_folding_formula = defs.iter().any(|d| d.de_mode != DeMode::Logarithmic);
    // Hybrids of power and folding formulas estimate distance best linearly.
    let de_mode = if has_power_formula && has_folding_formula {
        DeMode::Linear
    } else {
        DeMode::Auto
    };
    let iterations = match first.de_mode {
        DeMode::Box => 8,
        DeMode::Logarithmic => 12,
        _ => 14,
    } + rng.below(4);
    // Mixed hybrids escape late; a generous bailout keeps them intact.
    let bailout = defs.iter().map(|d| d.bailout).fold(0.0, f32::max);

    // Julia mode: mostly for power formulas, where it shines.
    let julia = has_power_formula && !first.vec4_state && rng.unit() < 0.3;
    Fractal {
        slots,
        iterations,
        bailout,
        de_mode,
        julia,
        julia_c: glam::Vec3::new(
            rng.range(-0.8, 0.8),
            rng.range(-0.8, 0.8),
            rng.range(-0.8, 0.8),
        ),
    }
}

/// A slightly altered copy: every parameter moves by up to `amount` of its
/// range, the Julia constant a little too.
pub fn mutate_fractal(fractal: &Fractal, library: &Library, seed: u64, amount: f32) -> Fractal {
    let mut rng = Rng::new(seed);
    let mut mutated = fractal.clone();
    for slot in &mut mutated.slots {
        let Some(def) = library.get(&slot.formula) else {
            continue;
        };
        for (value, param) in slot.params.iter_mut().zip(&def.params) {
            let span = param.max - param.min;
            *value = (*value + rng.range(-amount, amount) * span).clamp(param.min, param.max);
        }
    }
    if mutated.julia {
        for c in mutated.julia_c.as_mut() {
            *c += rng.range(-amount, amount);
        }
    }
    mutated
}

/// Small deterministic PRNG (SplitMix64).
pub(crate) struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    pub fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n.max(1))) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose;

    #[test]
    fn random_fractals_compose_and_respect_ranges() {
        let library = Library::builtin().unwrap();
        let mut hybrids = 0;
        for seed in 0..200 {
            let fractal = random_fractal(&library, seed);
            compose(&fractal, &library).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            assert!((1..=3).contains(&fractal.slots.len()));
            hybrids += usize::from(fractal.slots.len() > 1);
            for slot in &fractal.slots {
                assert_ne!(slot.formula, "pseudo_kleinian");
                let def = library.get(&slot.formula).unwrap();
                for (value, param) in slot.params.iter().zip(&def.params) {
                    assert!(
                        (param.min..=param.max).contains(value),
                        "{} {}",
                        def.id,
                        param.name
                    );
                }
            }
        }
        // Roughly half hybrids.
        assert!((60..=140).contains(&hybrids), "{hybrids} hybrids of 200");
    }

    #[test]
    fn random_is_deterministic_per_seed() {
        let library = Library::builtin().unwrap();
        assert_eq!(random_fractal(&library, 5), random_fractal(&library, 5));
        assert_ne!(random_fractal(&library, 5), random_fractal(&library, 6));
    }

    #[test]
    fn mutation_is_small_and_stays_in_range() {
        let library = Library::builtin().unwrap();
        let original = Fractal::default();
        let mutated = mutate_fractal(&original, &library, 1, 0.05);
        let (before, after) = (original.slots[0].params[0], mutated.slots[0].params[0]);
        assert_ne!(before, after);
        // Power ranges over 1..16: 5% is at most 0.75.
        assert!((before - after).abs() <= 0.75 + 1e-6);
        assert_eq!(mutated.slots[0].formula, original.slots[0].formula);
    }
}
