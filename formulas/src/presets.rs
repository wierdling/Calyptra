use glam::{DVec3, Vec3};
use scene::{Camera, DeMode, FormulaSlot, Fractal, Scene};

/// A ready-made fractal with a camera that frames it.
pub struct Preset {
    pub name: &'static str,
    pub fractal: Fractal,
    pub camera: Camera,
    /// Camera distance from the origin; sets the scale of distance-based
    /// settings (fog, ray length).
    pub view_distance: f64,
}

impl Preset {
    /// Loads the preset into `scene`, keeping shading and quality choices
    /// except those that depend on the fractal's size.
    pub fn apply(self, scene: &mut Scene) {
        let d = self.view_distance as f32;
        scene.fractal = self.fractal;
        scene.camera = self.camera;
        scene.quality.max_distance = (d * 8.0).max(20.0);
        // Fog grows with distance squared: keep the same look at any scale.
        scene.shading.fog_density = 0.15 / (d * d);
    }
}

fn preset(
    name: &'static str,
    slots: Vec<FormulaSlot>,
    iterations: u32,
    bailout: f32,
    distance: f64,
) -> Preset {
    // Three-quarter view from slightly above.
    let direction = DVec3::new(0.55, 0.45, 1.0).normalize();
    Preset {
        name,
        fractal: Fractal {
            slots,
            iterations,
            bailout,
            ..Default::default()
        },
        camera: Camera::looking_at(direction * distance, DVec3::ZERO, 50.0),
        view_distance: distance,
    }
}

/// Parameters left empty take the formula defaults (see `Library::normalize`).
pub fn presets() -> Vec<Preset> {
    let slot = |id: &str| FormulaSlot::new(id, vec![]);

    let mut juliabulb = preset("Juliabulb", vec![slot("mandelbulb")], 12, 2.0, 3.0);
    juliabulb.fractal.julia = true;
    juliabulb.fractal.julia_c = Vec3::new(0.35, -0.45, 0.2);

    let mut bulb_box = preset(
        "Hybrid: Mandelbulb + Mandelbox",
        vec![
            FormulaSlot::new("mandelbulb", vec![8.0]),
            FormulaSlot::new("mandelbox", vec![2.0, 0.5, 1.0, 1.0]),
        ],
        14,
        10.0,
        4.0,
    );
    bulb_box.fractal.de_mode = DeMode::Linear;

    let quaternion = preset(
        "Quaternion Julia",
        vec![slot("quaternion_julia")],
        12,
        4.0,
        3.2,
    );

    // An endless lattice: start inside it, looking down a corridor.
    let mut kleinian = preset(
        "Pseudo-Kleinian",
        vec![slot("pseudo_kleinian")],
        12,
        1000.0,
        1.0,
    );
    kleinian.camera = Camera::looking_at(
        DVec3::new(0.35, 0.05, 0.6),
        DVec3::new(-1.2, -0.25, 1.1),
        60.0,
    );
    // Fog and ray length for a scene seen from inside, not from a distance.
    kleinian.view_distance = 4.0;

    vec![
        preset("Mandelbulb", vec![slot("mandelbulb")], 12, 2.0, 2.8),
        juliabulb,
        preset("Mandelbox", vec![slot("mandelbox")], 15, 100.0, 9.0),
        preset("Menger sponge", vec![slot("menger")], 8, 1000.0, 3.5),
        preset(
            "KIFS octahedron",
            vec![slot("kifs_octahedron")],
            12,
            1000.0,
            4.0,
        ),
        preset("Amazing Surf", vec![slot("amazing_surf")], 12, 100.0, 14.0),
        bulb_box,
        quaternion,
        kleinian,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Library, compose};

    #[test]
    fn all_presets_compose() {
        let library = Library::builtin().unwrap();
        for preset in presets() {
            compose(&preset.fractal, &library).unwrap_or_else(|e| panic!("{}: {e}", preset.name));
        }
    }
}
