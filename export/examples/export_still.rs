//! Command-line still export, the same path as the app's Export window.
//!
//!     cargo run --release -p export --example export_still -- OUT.(png|exr) [WIDTH HEIGHT SAMPLES] [--scene SCENE.json|png|flame] [--png8]

use std::path::PathBuf;
use std::time::Instant;

use export::ImageFormat;
use render::{FlameRenderer, RaymarchRenderer, Renderer, StillSettings};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let scene_path = args.iter().position(|a| a == "--scene").map(|i| {
        args.remove(i);
        PathBuf::from(args.remove(i))
    });
    let png8 = args
        .iter()
        .position(|a| a == "--png8")
        .map(|i| args.remove(i))
        .is_some();
    let out = PathBuf::from(args.first().ok_or("usage: OUT [WIDTH HEIGHT SAMPLES]")?);
    let number = |i: usize, default: u32| args.get(i).map_or(Ok(default), |a| a.parse());
    let settings = StillSettings {
        width: number(1, 1920)?,
        height: number(2, 1080)?,
        samples: number(3, 16)?,
    };
    let format = match out.extension().and_then(|e| e.to_str()) {
        Some("exr") => ImageFormat::Exr,
        _ if png8 => ImageFormat::Png8,
        _ => ImageFormat::Png16,
    };

    let is_flame_file = |p: &PathBuf| {
        p.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("flame"))
    };
    let mut scene = match &scene_path {
        // Apophysis / JWildfire flames render with their own palette.
        Some(path) if is_flame_file(path) => {
            let imported = export::import_flames(path)?.remove(0);
            for warning in &imported.warnings {
                eprintln!("warning: {warning}");
            }
            let mut scene = scene::Scene {
                kind: scene::FractalKind::Flame,
                flame: imported.flame,
                ..Default::default()
            };
            scene.coloring.gradient = imported.gradient;
            scene
        }
        Some(path) => export::load_scene(path)?,
        None => scene::Scene::default(),
    };
    let library = formulas::Library::builtin()?;
    library.normalize(&mut scene.fractal);

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let mut renderer: Box<dyn Renderer> = match scene.kind {
        scene::FractalKind::Flame => Box::new(FlameRenderer::new(&device)),
        scene::FractalKind::Distance => {
            let mut raymarch = RaymarchRenderer::new(&device);
            raymarch.set_de_source(&device, formulas::compose(&scene.fractal, &library)?);
            if let Some(error) = raymarch.error() {
                return Err(error.into());
            }
            Box::new(raymarch)
        }
    };

    let start = Instant::now();
    let mut last_report = 0.0;
    let image = render::render_still(&device, &queue, renderer.as_mut(), &scene, settings, |f| {
        if f - last_report >= 0.1 {
            last_report = f;
            eprintln!("{:3.0}%", f * 100.0);
        }
        true
    })
    .ok_or("cancelled")?;
    let rendered = start.elapsed();
    export::save_image(&out, &image, format, &scene)?;
    println!(
        "{}x{} @ {} spp: rendered in {:.1?}, saved {} in {:.1?}",
        settings.width,
        settings.height,
        settings.samples,
        rendered,
        out.display(),
        start.elapsed() - rendered
    );
    Ok(())
}
