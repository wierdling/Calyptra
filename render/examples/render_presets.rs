//! Headless renders for checking formulas and palettes by eye.
//!
//!     cargo run -p render --example render_presets              # each preset → renders/presets/*.png
//!     cargo run -p render --example render_presets -- --palettes  # one contact sheet of all palettes

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
    if palettes {
        palette_sheet(&gpu, &out_dir)
    } else {
        each_preset(&gpu, &out_dir.join("presets"))
    }
}

fn each_preset(gpu: &Gpu, out_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out_dir)?;
    let (width, height) = (960, 600);
    let mut renderer = RaymarchRenderer::new(&gpu.device);
    for preset in formulas::presets() {
        let name = preset.name;
        let mut scene = scene::Scene::default();
        preset.apply(&mut scene);
        let pixels = render(gpu, &mut renderer, &scene, width, height)?;
        let file_name = name
            .to_lowercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
        let path = out_dir.join(format!("{file_name}.png"));
        write_png(&path, width, height, &pixels)?;
        println!("{name} -> {}", path.display());
    }
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
    viewport.render(&gpu.device, &gpu.queue, renderer, &scene);
    for _ in 0..2 {
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        renderer.poll(&gpu.device);
    }
    if let Some((lo, hi)) = renderer.probe().and_then(|p| p.color_range) {
        scene.coloring.fit_to_range(lo, hi);
    }

    // Then the real render through the export path (tiled, 4 samples).
    let settings = StillSettings {
        width,
        height,
        samples: 4,
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
