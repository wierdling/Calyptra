//! Renders every formula preset headlessly to `renders/presets/*.png`.
//!
//!     cargo run -p render --example render_presets [width height]

use render::{DisplaySettings, RaymarchRenderer, Viewport};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1).map(|a| a.parse::<u32>());
    let width = args.next().transpose()?.unwrap_or(960);
    let height = args.next().transpose()?.unwrap_or(600);

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

    let library = formulas::Library::builtin()?;
    let mut renderer = RaymarchRenderer::new(&device);
    let mut viewport = Viewport::new(&device, &queue, width, height);
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../renders/presets");
    std::fs::create_dir_all(&out_dir)?;

    for preset in formulas::presets() {
        let name = preset.name;
        let mut scene = scene::Scene::default();
        preset.apply(&mut scene);
        library.normalize(&mut scene.fractal);
        renderer.set_de_source(&device, formulas::compose(&scene.fractal, &library)?);
        if let Some(error) = renderer.error() {
            return Err(format!("{name}: {error}").into());
        }
        viewport.resize(&device, width, height);
        viewport.render(
            &device,
            &queue,
            &mut renderer,
            &scene,
            &DisplaySettings::default(),
        );
        let pixels = viewport.read_display_pixels(&device, &queue);

        let file_name = name
            .to_lowercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
        let path = out_dir.join(format!("{file_name}.png"));
        let mut encoder = png::Encoder::new(std::fs::File::create(&path)?, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder.write_header()?.write_image_data(&pixels)?;
        println!("{name} -> {}", path.display());
    }
    Ok(())
}
