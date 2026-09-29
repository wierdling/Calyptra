//! Random 3D fractals that are worth looking at.
//!
//! Most random parameter sets give an empty screen, a featureless blob, or
//! noise. Each candidate is therefore rendered at thumbnail size with the
//! denoiser-guide pass (normal + depth per pixel), measured, and framed:
//! the camera backs off or closes in and re-aims until the fractal fills
//! most of the view. The first candidate that passes is kept, otherwise
//! the best of the attempts.

use glam::DVec3;
use scene::{Camera, ColorSource, FractalKind, Scene, Wrap};

use crate::still::read_texture;
use crate::{AUX_FORMAT, FrameInput, HdrTarget, RaymarchRenderer, Region, Renderer};

/// Thumbnail used to judge candidates.
const PROBE_WIDTH: u32 = 128;
const PROBE_HEIGHT: u32 = 96;
pub const MAX_ATTEMPTS: u32 = 24;
/// Fraction of the frame the framed fractal should span.
const TARGET_EXTENT: f32 = 0.8;

/// What a thumbnail render of a candidate shows.
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewStats {
    /// Fraction of pixels that hit the surface.
    pub coverage: f32,
    /// Mean normal change between neighboring hit pixels: ~0 for smooth
    /// blobs, high for noise.
    pub detail: f32,
    /// Larger of the hit bounding box's width and height, as a fraction of
    /// the frame.
    pub extent: f32,
    /// Fraction of the frame's border pixels that hit: the fractal is cut
    /// off by the frame edges.
    pub border: f32,
    /// 1 − length of the mean surface normal: ~0 for a flat sheet facing
    /// one way, large for rounded or intricate shapes.
    pub spread: f32,
    /// World-space centroid of the visible surface.
    pub centroid: Option<DVec3>,
}

impl ViewStats {
    /// Worth showing: fills a fair share of the frame, has structure, and
    /// is not badly cut off.
    pub fn acceptable(&self) -> bool {
        (0.08..=0.8).contains(&self.coverage)
            && (0.012..=0.45).contains(&self.detail)
            && self.spread > 0.1
            && self.border < 0.25
    }

    /// Higher is better; ranks candidates when none is acceptable.
    pub fn score(&self) -> f32 {
        if self.coverage <= 0.0 {
            return f32::NEG_INFINITY;
        }
        -(self.coverage - 0.35).abs() - 0.3 * (self.detail / 0.12).ln().abs() - self.border
            + self.spread.min(0.5)
    }
}

/// Renders `scene`'s guides at thumbnail size and measures them.
pub fn view_stats(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut RaymarchRenderer,
    scene: &Scene,
) -> ViewStats {
    let (w, h) = (PROBE_WIDTH, PROBE_HEIGHT);
    let albedo = HdrTarget::with_format(device, w, h, AUX_FORMAT);
    let normal_depth = HdrTarget::with_format(device, w, h, AUX_FORMAT);
    let input = FrameInput {
        scene,
        frame: 0,
        sample: 0,
        region: Region::full(w, h),
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    if !renderer.render_aux(device, queue, &mut encoder, &albedo, &normal_depth, &input) {
        return ViewStats::default();
    }
    queue.submit([encoder.finish()]);
    measure(
        &read_texture(device, queue, &normal_depth.texture),
        &scene.camera,
        w,
        h,
    )
}

fn measure(pixels: &[[f32; 4]], camera: &Camera, w: u32, h: u32) -> ViewStats {
    let hit = |x: u32, y: u32| pixels[(y * w + x) as usize][3] >= 0.0;
    let tan_half = (camera.fov_y_degrees.to_radians() * 0.5).tan();
    let aspect = f64::from(w) / f64::from(h);
    let (right, up, forward) = (camera.right(), camera.up(), camera.forward());

    let (mut hits, mut detail_sum, mut detail_pairs) = (0u32, 0.0f32, 0u32);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0, 0);
    let mut centroid = DVec3::ZERO;
    let mut normal_sum = [0.0f32; 3];
    for y in 0..h {
        for x in 0..w {
            if !hit(x, y) {
                continue;
            }
            hits += 1;
            (min_x, min_y, max_x, max_y) = (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y));
            let [nx, ny, nz, t] = pixels[(y * w + x) as usize];
            // The same ray the shader cast through this pixel's center.
            let u = (f64::from(x) + 0.5) / f64::from(w) * 2.0 - 1.0;
            let v = 1.0 - (f64::from(y) + 0.5) / f64::from(h) * 2.0;
            let dir = (forward + right * (u * tan_half * aspect) + up * (v * tan_half)).normalize();
            centroid += camera.position + dir * f64::from(t);
            normal_sum = [normal_sum[0] + nx, normal_sum[1] + ny, normal_sum[2] + nz];
            if x + 1 < w && hit(x + 1, y) {
                let [mx, my, mz, _] = pixels[(y * w + x + 1) as usize];
                detail_sum += 1.0 - (nx * mx + ny * my + nz * mz).clamp(-1.0, 1.0);
                detail_pairs += 1;
            }
        }
    }
    if hits == 0 {
        return ViewStats::default();
    }
    let border_pixels = 2 * (w + h) - 4;
    let border_hits = (0..w)
        .flat_map(|x| [(x, 0), (x, h - 1)])
        .chain((1..h - 1).flat_map(|y| [(0, y), (w - 1, y)]))
        .filter(|&(x, y)| hit(x, y))
        .count();
    ViewStats {
        coverage: hits as f32 / (w * h) as f32,
        detail: detail_sum / detail_pairs.max(1) as f32,
        extent: ((max_x - min_x + 1) as f32 / w as f32).max((max_y - min_y + 1) as f32 / h as f32),
        border: border_hits as f32 / border_pixels as f32,
        centroid: Some(centroid / f64::from(hits)),
        spread: 1.0
            - normal_sum
                .map(|c| c / hits as f32)
                .iter()
                .map(|c| c * c)
                .sum::<f32>()
                .sqrt(),
    }
}

/// Moves the camera along `direction` (and re-aims at the fractal) until
/// it fills about [`TARGET_EXTENT`] of the frame. Returns the final stats,
/// or `None` if nothing is visible from any distance tried.
fn frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut RaymarchRenderer,
    scene: &mut Scene,
    direction: DVec3,
) -> Option<ViewStats> {
    let mut target = DVec3::ZERO;
    let mut distance = 4.0;
    let mut stats = ViewStats::default();
    let mut seen = false;
    for step in 0..7 {
        scene.camera = Camera::looking_at(target + direction * distance, target, 50.0);
        scene.quality.max_distance = (distance as f32 * 8.0).max(20.0);
        stats = view_stats(device, queue, renderer, scene);

        if stats.coverage == 0.0 {
            // Nothing: maybe the camera is inside something hollow, or the
            // fractal is huge. Back off; give up if it never shows.
            if seen || step >= 2 {
                return None;
            }
            distance *= 3.0;
            continue;
        }
        seen = true;
        if stats.coverage > 0.95 {
            // Inside it, or far too close.
            distance *= 2.5;
            continue;
        }
        // Aim at what is actually there (fractals are not always centered).
        if step < 3
            && let Some(centroid) = stats.centroid
        {
            target = centroid;
        }
        let framed = (TARGET_EXTENT - 0.15..=TARGET_EXTENT + 0.15).contains(&stats.extent);
        if framed && stats.border < 0.25 {
            break;
        }
        // Too big (or cut off): back away; too small: close in.
        let factor = if stats.border >= 0.25 {
            1.6
        } else {
            f64::from(stats.extent / TARGET_EXTENT)
        };
        distance *= factor.clamp(0.4, 2.5);
    }
    Some(stats)
}

/// A finished random candidate.
pub struct RandomResult {
    pub scene: Scene,
    pub stats: ViewStats,
    /// Attempts used (1-based).
    pub attempts: u32,
    /// `false` if no candidate passed and this is the best of the rest.
    pub accepted: bool,
}

/// Searches for a random 3D fractal worth showing. `base` supplies what
/// is kept (lighting, render and display settings). `progress` gets the
/// attempt number and returns `false` to cancel.
pub fn find_random_fractal(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    library: &formulas::Library,
    base: &Scene,
    seed: u64,
    mut progress: impl FnMut(u32) -> bool,
) -> Option<RandomResult> {
    let mut renderer = RaymarchRenderer::new(device);
    let mut best: Option<(f32, Scene, ViewStats, u32)> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        if !progress(attempt) {
            return None;
        }
        let candidate_seed =
            seed.wrapping_add(u64::from(attempt).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut rng = Rng(candidate_seed ^ 0xC0FF_EE00);
        let mut scene = base.clone();
        scene.kind = FractalKind::Distance;
        scene.fractal = formulas::random_fractal(library, candidate_seed);
        library.normalize(&mut scene.fractal);
        let Ok(de) = formulas::compose(&scene.fractal, library) else {
            continue;
        };
        renderer.set_de_source(device, de);
        if renderer.error().is_some() {
            continue;
        }

        // A three-quarter view from a random side, slightly above.
        let direction = DVec3::new(
            rng.range(-1.0, 1.0),
            rng.range(0.15, 0.7),
            rng.range(-1.0, 1.0),
        )
        .normalize_or(DVec3::Z);
        let Some(stats) = frame(device, queue, &mut renderer, &mut scene, direction) else {
            continue;
        };
        if stats.acceptable() {
            return Some(finish(scene, stats, attempt, true, &mut rng));
        }
        if best
            .as_ref()
            .is_none_or(|(score, ..)| stats.score() > *score)
        {
            best = Some((stats.score(), scene, stats, attempt));
        }
    }
    best.map(|(_, scene, stats, attempt)| {
        let mut rng = Rng(seed ^ 0xBE57);
        finish(scene, stats, attempt, false, &mut rng)
    })
}

/// Random palette and coloring; fog scaled to the framing distance.
fn finish(
    mut scene: Scene,
    stats: ViewStats,
    attempts: u32,
    accepted: bool,
    rng: &mut Rng,
) -> RandomResult {
    let palettes = color::presets();
    let palette = &palettes[rng.below(palettes.len() as u32) as usize];
    scene.coloring.gradient = palette.gradient.clone();
    scene.coloring.wrap = if palette.cyclic {
        Wrap::Repeat
    } else {
        Wrap::Mirror
    };
    scene.coloring.source = match rng.unit() {
        u if u < 0.4 => ColorSource::OrbitTrap,
        u if u < 0.65 => ColorSource::PlaneTrap,
        u if u < 0.85 => ColorSource::Iterations,
        _ => ColorSource::Normal,
    };
    // The caller fits offset/frequency to the view; start neutral.
    scene.coloring.offset = 0.0;
    scene.coloring.frequency = 1.0;

    let distance = stats
        .centroid
        .map_or(4.0, |c| c.distance(scene.camera.position)) as f32;
    scene.shading.fog_density = 0.15 / (distance * distance);
    scene.camera.focus_distance = f64::from(distance);
    RandomResult {
        scene,
        stats,
        attempts,
        accepted,
    }
}

/// Small deterministic PRNG (SplitMix64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n.max(1))) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, hit: impl Fn(u32, u32) -> bool) -> Vec<[f32; 4]> {
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if hit(x, y) {
                    // Normals vary with x: some detail.
                    let a = x as f32 * 0.3;
                    [a.sin(), 0.0, a.cos(), 3.0]
                } else {
                    [0.0, 0.0, 0.0, -1.0]
                }
            })
            .collect()
    }

    #[test]
    fn measures_coverage_extent_and_border() {
        let camera = Camera::default();
        // A centered block covering the middle half in each direction.
        let pixels = image(40, 20, |x, y| (10..30).contains(&x) && (5..15).contains(&y));
        let stats = measure(&pixels, &camera, 40, 20);
        assert!((stats.coverage - 0.25).abs() < 1e-6);
        assert!((stats.extent - 0.5).abs() < 1e-6);
        assert_eq!(stats.border, 0.0);
        assert!(stats.detail > 0.0);
        assert!(stats.centroid.is_some());
        assert!(stats.spread > 0.0 && stats.spread < 1.0);
    }

    #[test]
    fn flat_sheets_are_rejected() {
        let camera = Camera::default();
        // Every hit faces the same way: a slab seen face-on.
        let pixels: Vec<[f32; 4]> = (0..40 * 20)
            .map(|i| {
                let (x, y) = (i % 40, i / 40);
                if (10..30).contains(&x) && (5..15).contains(&y) {
                    [0.0, 0.0, 1.0, 3.0]
                } else {
                    [0.0, 0.0, 0.0, -1.0]
                }
            })
            .collect();
        let stats = measure(&pixels, &camera, 40, 20);
        assert!(stats.spread < 0.01);
        assert!(!stats.acceptable());
    }

    #[test]
    fn empty_and_full_views_are_rejected() {
        let camera = Camera::default();
        let empty = measure(&image(20, 10, |_, _| false), &camera, 20, 10);
        assert!(!empty.acceptable());
        let full = measure(&image(20, 10, |_, _| true), &camera, 20, 10);
        assert!(!full.acceptable());
        assert_eq!(full.border, 1.0);
    }
}
