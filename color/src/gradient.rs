use serde::{Deserialize, Serialize};

use crate::{linear_to_oklab, linear_to_srgb8, oklab_to_linear, srgb8_to_linear};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    /// Position in [0, 1].
    pub position: f32,
    /// sRGB color.
    pub color: [u8; 3],
}

impl Stop {
    pub fn new(position: f32, color: [u8; 3]) -> Self {
        Self { position, color }
    }
}

/// A color gradient over [0, 1], interpolated in OKLab.
///
/// Stops need not be sorted (the editor reorders them freely); two stops at
/// the same position make a hard edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub stops: Vec<Stop>,
}

impl Default for Gradient {
    fn default() -> Self {
        crate::presets()
            .into_iter()
            .next()
            .expect("at least one preset")
            .gradient
    }
}

impl Gradient {
    /// Evenly spaced stops from a list of `0xRRGGBB` colors.
    pub fn from_hex(colors: &[u32]) -> Self {
        let last = (colors.len().max(2) - 1) as f32;
        Self {
            stops: colors
                .iter()
                .enumerate()
                .map(|(i, &hex)| {
                    Stop::new(
                        i as f32 / last,
                        [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8],
                    )
                })
                .collect(),
        }
    }

    fn sorted_stops(&self) -> Vec<Stop> {
        let mut stops = self.stops.clone();
        stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        stops
    }

    /// Linear RGB at `t` (clamped to [0, 1]).
    pub fn sample_linear(&self, t: f32) -> [f32; 3] {
        sample_sorted(&self.sorted_stops(), t)
    }

    pub fn sample_srgb8(&self, t: f32) -> [u8; 3] {
        linear_to_srgb8(self.sample_linear(t))
    }

    /// `n` evenly spaced sRGB samples covering [0, 1] — the GPU lookup table.
    pub fn bake_srgb8(&self, n: usize) -> Vec<[u8; 3]> {
        let stops = self.sorted_stops();
        let last = (n.max(2) - 1) as f32;
        (0..n)
            .map(|i| linear_to_srgb8(sample_sorted(&stops, i as f32 / last)))
            .collect()
    }

    pub fn reverse(&mut self) {
        for stop in &mut self.stops {
            stop.position = 1.0 - stop.position;
        }
        self.stops.reverse();
    }

    /// Keeps the order of colors but spaces the stops evenly.
    pub fn distribute_evenly(&mut self) {
        self.stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        let last = (self.stops.len().max(2) - 1) as f32;
        for (i, stop) in self.stops.iter_mut().enumerate() {
            stop.position = i as f32 / last;
        }
    }

    /// Removes stops that the remaining ones reproduce to within `tolerance`
    /// (OKLab distance). Used to tame 256-entry imported palettes.
    pub fn simplify(&mut self, tolerance: f32) {
        let mut stops = self.sorted_stops();
        loop {
            let mut best: Option<(usize, f32)> = None;
            for i in 1..stops.len().saturating_sub(1) {
                let (prev, stop, next) = (stops[i - 1], stops[i], stops[i + 1]);
                // Keep hard edges and anything the neighbors can't reproduce.
                if stop.position <= prev.position || stop.position >= next.position {
                    continue;
                }
                let predicted = sample_sorted(&[prev, next], stop.position);
                let error = oklab_distance(predicted, srgb8_to_linear(stop.color));
                if error < tolerance && best.is_none_or(|(_, e)| error < e) {
                    best = Some((i, error));
                }
            }
            match best {
                Some((i, _)) => {
                    stops.remove(i);
                }
                None => break,
            }
        }
        self.stops = stops;
    }
}

fn oklab_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (a, b) = (linear_to_oklab(a), linear_to_oklab(b));
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn sample_sorted(stops: &[Stop], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return [0.0; 3];
    };
    if t <= first.position {
        return srgb8_to_linear(first.color);
    }
    if t >= last.position {
        return srgb8_to_linear(last.color);
    }
    // First stop strictly after t; the one before it is at or before t.
    let upper = stops
        .partition_point(|s| s.position <= t)
        .min(stops.len() - 1);
    let (a, b) = (stops[upper - 1], stops[upper]);
    let span = b.position - a.position;
    let f = if span > 0.0 {
        (t - a.position) / span
    } else {
        1.0
    };
    let (la, lb) = (
        linear_to_oklab(srgb8_to_linear(a.color)),
        linear_to_oklab(srgb8_to_linear(b.color)),
    );
    let mixed = [0, 1, 2].map(|i| la[i] + (lb[i] - la[i]) * f);
    oklab_to_linear(mixed).map(|c| c.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_and_clamping() {
        let g = Gradient::from_hex(&[0x000000, 0xffffff]);
        assert_eq!(g.sample_srgb8(0.0), [0, 0, 0]);
        assert_eq!(g.sample_srgb8(1.0), [255, 255, 255]);
        assert_eq!(g.sample_srgb8(-3.0), [0, 0, 0]);
        assert_eq!(g.sample_srgb8(7.0), [255, 255, 255]);
    }

    #[test]
    fn oklab_midpoint_is_perceptual_gray() {
        // OKLab L is perceptual lightness: L = 0.5 is linear 0.125, which
        // encodes to sRGB ~99. (A naive sRGB lerp gives 128, a linear lerp 188.)
        let mid = Gradient::from_hex(&[0x000000, 0xffffff]).sample_srgb8(0.5);
        assert!((95..=105).contains(&mid[0]), "{mid:?}");
    }

    #[test]
    fn unsorted_stops_sample_in_position_order() {
        let g = Gradient {
            stops: vec![Stop::new(1.0, [0, 0, 255]), Stop::new(0.0, [255, 0, 0])],
        };
        assert_eq!(g.sample_srgb8(0.0), [255, 0, 0]);
        assert_eq!(g.sample_srgb8(1.0), [0, 0, 255]);
    }

    #[test]
    fn coincident_stops_make_a_hard_edge() {
        let g = Gradient {
            stops: vec![
                Stop::new(0.0, [255, 0, 0]),
                Stop::new(0.5, [255, 0, 0]),
                Stop::new(0.5, [0, 0, 255]),
                Stop::new(1.0, [0, 0, 255]),
            ],
        };
        assert_eq!(g.sample_srgb8(0.49), [255, 0, 0]);
        assert_eq!(g.sample_srgb8(0.51), [0, 0, 255]);
    }

    #[test]
    fn reverse_mirrors() {
        let mut g = Gradient::from_hex(&[0xff0000, 0x00ff00, 0x0000ff]);
        g.reverse();
        assert_eq!(g.sample_srgb8(0.0), [0, 0, 255]);
        assert_eq!(g.sample_srgb8(1.0), [255, 0, 0]);
    }

    #[test]
    fn simplify_drops_redundant_stops_only() {
        // 256 samples of a two-stop gradient collapse back to (about) two.
        let two = Gradient::from_hex(&[0x102030, 0xf0e0a0]);
        let mut dense = Gradient {
            stops: (0..256)
                .map(|i| {
                    let t = i as f32 / 255.0;
                    Stop::new(t, two.sample_srgb8(t))
                })
                .collect(),
        };
        dense.simplify(0.01);
        assert!(dense.stops.len() <= 4, "{} stops left", dense.stops.len());

        // A sharp color change must survive.
        let mut kept = Gradient::from_hex(&[0x000000, 0xff0000, 0x000000]);
        kept.simplify(0.01);
        assert_eq!(kept.stops.len(), 3);
    }
}
