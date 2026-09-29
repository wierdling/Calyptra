//! Headless renders for checking formulas and palettes by eye.
//!
//!     cargo run -p render --example render_presets              # each preset → renders/presets/*.png
//!     cargo run -p render --example render_presets -- --palettes  # one contact sheet of all palettes
//!     cargo run -p render --example render_presets -- --pathtrace # path traced, 64 spp, denoised

use std::path::Path;

use render::{
    RaymarchRenderer, Renderer, StillSettings, Viewport, display_transform, render_still,
};

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let palettes = std::env::args().any(|a| a == "--palettes");
    let path_trace = std::env::args().any(|a| a == "--pathtrace");
    let flames = std::env::args().any(|a| a == "--flames");
    let random = std::env::args().any(|a| a == "--random");

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))?;
    println!("GPU: {}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let gpu = Gpu { device, queue };

    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../renders");
    if random {
        random_sheet(&gpu, &out_dir)
    } else if flames {
        flame_sheet(&gpu, &out_dir)
    } else if palettes {
        palette_sheet(&gpu, &out_dir)
    } else {
        each_preset(&gpu, &out_dir.join("presets"), path_trace)
    }
}

fn each_preset(
    gpu: &Gpu,
    out_dir: &Path,
    path_trace: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out_dir)?;
    let (width, height) = (960, 600);
    let mut renderer = RaymarchRenderer::new(&gpu.device);
    for preset in formulas::presets() {
        let name = preset.name;
        let mut scene = scene::Scene::default();
        preset.apply(&mut scene);
        if path_trace {
            scene.render.mode = scene::RenderMode::PathTrace;
        }
        let start = std::time::Instant::now();
        let pixels = render(gpu, &mut renderer, &scene, width, height)?;
        let elapsed = start.elapsed();
        let file_name = name
            .to_lowercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
        let suffix = if path_trace { "_pt" } else { "" };
        let path = out_dir.join(format!("{file_name}{suffix}.png"));
        write_png(&path, width, height, &pixels)?;
        println!("{name} ({elapsed:.1?}) -> {}", path.display());
    }
    Ok(())
}

/// Measures the presets (known good), then runs the random search for 12
/// seeds and tiles the results.
fn random_sheet(gpu: &Gpu, out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out_dir)?;
    let library = formulas::Library::builtin()?;
    let mut renderer = RaymarchRenderer::new(&gpu.device);
    println!("presets:");
    for preset in formulas::presets() {
        let name = preset.name;
        let mut scene = scene::Scene::default();
        preset.apply(&mut scene);
        library.normalize(&mut scene.fractal);
        renderer.set_de_source(&gpu.device, formulas::compose(&scene.fractal, &library)?);
        let stats = render::view_stats(&gpu.device, &gpu.queue, &mut renderer, &scene);
        println!("  {name:32} {}", describe(&stats));
    }

    let (tile_w, tile_h, columns) = (400u32, 300u32, 4u32);
    let seeds: Vec<u64> = (1..=12).collect();
    let rows = (seeds.len() as u32).div_ceil(columns);
    let (sheet_w, sheet_h) = (tile_w * columns, tile_h * rows);
    let mut sheet = vec![0u8; (sheet_w * sheet_h * 4) as usize];
    println!("random:");
    for (index, &seed) in seeds.iter().enumerate() {
        let start = std::time::Instant::now();
        let result = render::find_random_fractal(
            &gpu.device,
            &gpu.queue,
            &library,
            &scene::Scene::default(),
            seed,
            |_| true,
        )
        .ok_or("no candidate")?;
        let formulas: Vec<&str> = result
            .scene
            .fractal
            .slots
            .iter()
            .map(|s| s.formula.as_str())
            .collect();
        println!(
            "  seed {seed:2}: {} after {:2} tries in {:.1?} {} {:?}",
            if result.accepted { "ok  " } else { "best" },
            result.attempts,
            start.elapsed(),
            describe(&result.stats),
            formulas,
        );
        let tile = render(gpu, &mut renderer, &result.scene, tile_w, tile_h)?;
        let (x0, y0) = (
            (index as u32 % columns) * tile_w,
            (index as u32 / columns) * tile_h,
        );
        for row in 0..tile_h {
            let src = (row * tile_w * 4) as usize;
            let dst = (((y0 + row) * sheet_w + x0) * 4) as usize;
            sheet[dst..dst + (tile_w * 4) as usize]
                .copy_from_slice(&tile[src..src + (tile_w * 4) as usize]);
        }
    }
    let path = out_dir.join("random.png");
    write_png(&path, sheet_w, sheet_h, &sheet)?;
    println!("-> {}", path.display());
    Ok(())
}

fn describe(stats: &render::ViewStats) -> String {
    format!(
        "coverage {:.2} detail {:.3} spread {:.2} extent {:.2} border {:.2}",
        stats.coverage, stats.detail, stats.spread, stats.extent, stats.border
    )
}

/// Flame presets followed by random flames, each with a different palette,
/// tiled 4 across.
fn flame_sheet(gpu: &Gpu, out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut flames: Vec<(String, scene::flame::Flame)> = scene::flame::presets()
        .into_iter()
        .map(|(name, flame)| (name.to_owned(), flame))
        .collect();
    for seed in 1..=8u64 {
        flames.push((format!("random {seed}"), scene::flame::Flame::random(seed)));
    }
    tile_flames(gpu, out_dir, "flames.png", flames)?;
    let flames_3d = (1..=12u64)
        .map(|seed| {
            (
                format!("random 3D {seed}"),
                scene::flame::Flame::random_3d(seed),
            )
        })
        .collect();
    tile_flames(gpu, out_dir, "flames_3d.png", flames_3d)?;

    // A 3D flame with depth of field, larger.
    let mut flame = scene::flame::Flame::random_3d(3);
    flame.camera.depth_of_field = 0.08;
    flame.camera.focus_depth = 0.3;
    let mut scene = scene::Scene {
        kind: scene::FractalKind::Flame,
        flame,
        ..Default::default()
    };
    scene.coloring.gradient = color::presets()[2].gradient.clone();
    let (width, height) = (960, 540);
    let settings = StillSettings {
        width,
        height,
        samples: 1,
    };
    let mut renderer = render::FlameRenderer::new(&gpu.device);
    let image = render_still(
        &gpu.device,
        &gpu.queue,
        &mut renderer,
        &scene,
        settings,
        |_| true,
    )
    .ok_or("cancelled")?;
    let pixels: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|&[r, g, b, _]| {
            let rgb = display_transform([r, g, b], &scene.display);
            [rgb[0], rgb[1], rgb[2], 1.0].map(|c| (c * 255.0).round() as u8)
        })
        .collect();
    let path = out_dir.join("flame_3d_dof.png");
    write_png(&path, width, height, &pixels)?;
    println!("-> {}", path.display());
    Ok(())
}

fn tile_flames(
    gpu: &Gpu,
    out_dir: &Path,
    file_name: &str,
    flames: Vec<(String, scene::flame::Flame)>,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out_dir)?;
    let (tile_w, tile_h, columns) = (400u32, 300u32, 4u32);
    let palettes = color::presets();
    let rows = (flames.len() as u32).div_ceil(columns);
    let (sheet_w, sheet_h) = (tile_w * columns, tile_h * rows);
    let mut sheet = vec![0u8; (sheet_w * sheet_h * 4) as usize];

    let mut renderer = render::FlameRenderer::new(&gpu.device);
    for (index, (name, flame)) in flames.into_iter().enumerate() {
        let mut scene = scene::Scene {
            kind: scene::FractalKind::Flame,
            flame,
            ..Default::default()
        };
        let palette = &palettes[(index + 1) % palettes.len()];
        scene.coloring.gradient = palette.gradient.clone();
        let start = std::time::Instant::now();
        let settings = StillSettings {
            width: tile_w,
            height: tile_h,
            samples: 1,
        };
        let image = render_still(
            &gpu.device,
            &gpu.queue,
            &mut renderer,
            &scene,
            settings,
            |_| true,
        )
        .ok_or("cancelled")?;
        let (x0, y0) = (
            (index as u32 % columns) * tile_w,
            (index as u32 / columns) * tile_h,
        );
        for (i, &[r, g, b, _]) in image.pixels.iter().enumerate() {
            let (x, y) = (i as u32 % tile_w, i as u32 / tile_w);
            let rgb = display_transform([r, g, b], &scene.display);
            let dst = (((y0 + y) * sheet_w + x0 + x) * 4) as usize;
            for (c, value) in rgb.iter().enumerate() {
                sheet[dst + c] = (value * 255.0).round() as u8;
            }
            sheet[dst + 3] = 255;
        }
        println!(
            "{index:2}: {name} ({}, {:.1?})",
            palette.name,
            start.elapsed()
        );
    }
    // The interactive path: progressive batches through a Viewport.
    let scene = scene::Scene {
        kind: scene::FractalKind::Flame,
        ..Default::default()
    };
    let mut viewport = Viewport::new(&gpu.device, &gpu.queue, 640, 360);
    for sample in 0..10 {
        viewport.render(&gpu.device, &gpu.queue, &mut renderer, &scene, sample);
    }
    let pixels = viewport.read_display_pixels(&gpu.device, &gpu.queue);
    let lit = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] > 40 || p[1] > 40 || p[2] > 40)
        .count();
    println!("viewport: {} samples, {lit} lit pixels", viewport.samples());
    write_png(&out_dir.join("flame_viewport.png"), 640, 360, &pixels)?;

    let path = out_dir.join(file_name);
    write_png(&path, sheet_w, sheet_h, &sheet)?;
    println!("-> {}", path.display());
    Ok(())
}

/// Every palette on the default Mandelbulb, tiled 4 across.
fn palette_sheet(gpu: &Gpu, out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out_dir)?;
    let (tile_w, tile_h, columns) = (400u32, 300u32, 4u32);
    let palettes = color::presets();
    let rows = (palettes.len() as u32).div_ceil(columns);
    let (sheet_w, sheet_h) = (tile_w * columns, tile_h * rows);
    let mut sheet = vec![0u8; (sheet_w * sheet_h * 4) as usize];

    let mut renderer = RaymarchRenderer::new(&gpu.device);
    for (index, palette) in palettes.into_iter().enumerate() {
        let mut scene = scene::Scene::default();
        scene.coloring.gradient = palette.gradient;
        if !palette.cyclic {
            scene.coloring.wrap = scene::Wrap::Mirror;
        }
        let tile = render(gpu, &mut renderer, &scene, tile_w, tile_h)?;
        let (x0, y0) = (
            (index as u32 % columns) * tile_w,
            (index as u32 / columns) * tile_h,
        );
        for row in 0..tile_h {
            let src = (row * tile_w * 4) as usize;
            let dst = (((y0 + row) * sheet_w + x0) * 4) as usize;
            sheet[dst..dst + (tile_w * 4) as usize]
                .copy_from_slice(&tile[src..src + (tile_w * 4) as usize]);
        }
        println!("{index:2}: {}", palette.name);
    }
    let path = out_dir.join("palettes.png");
    write_png(&path, sheet_w, sheet_h, &sheet)?;
    println!("-> {}", path.display());
    Ok(())
}

fn render(
    gpu: &Gpu,
    renderer: &mut RaymarchRenderer,
    scene: &scene::Scene,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let library = formulas::Library::builtin()?;
    let mut scene = scene.clone();
    library.normalize(&mut scene.fractal);
    renderer.set_de_source(&gpu.device, formulas::compose(&scene.fractal, &library)?);
    if let Some(error) = renderer.error() {
        return Err(error.into());
    }

    // Measure first: fit the gradient to the visible values, as the app does.
    let mut viewport = Viewport::new(&gpu.device, &gpu.queue, width, height);
    viewport.render(&gpu.device, &gpu.queue, renderer, &scene, 0);
    for _ in 0..2 {
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        renderer.poll(&gpu.device);
    }
    if let Some((lo, hi)) = renderer.probe().and_then(|p| p.color_range) {
        scene.coloring.fit_to_range(lo, hi);
    }

    // Then the real render through the export path (tiled).
    let samples = match scene.render.mode {
        scene::RenderMode::Preview => 4,
        scene::RenderMode::PathTrace => 64,
    };
    let settings = StillSettings {
        width,
        height,
        samples,
    };
    let image = render_still(&gpu.device, &gpu.queue, renderer, &scene, settings, |_| {
        true
    })
    .ok_or("cancelled")?;
    Ok(image
        .pixels
        .iter()
        .flat_map(|&[r, g, b, _]| {
            let [r, g, b] = display_transform([r, g, b], &scene.display);
            [r, g, b]
                .map(|c| (c * 255.0).round() as u8)
                .into_iter()
                .chain([255])
        })
        .collect())
}

fn write_png(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(std::fs::File::create(path)?, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}
