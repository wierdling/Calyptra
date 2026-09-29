#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod camera_control;
mod color_ui;
mod export_ui;
mod flame_ui;
mod formulas_ui;
mod fractal_ui;
mod history;
mod movie_ui;
mod random_job;
mod timeline_ui;

use std::sync::Arc;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,wgpu_hal=error,fractals=info"),
    )
    .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Fractals")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: wgpu_options(),
        ..Default::default()
    };
    eframe::run_native(
        "Fractals",
        options,
        Box::new(|cc| Ok(Box::new(app::FractalApp::new(cc)?))),
    )
}

/// D3D12/Vulkan only; timestamp queries and the full storage-buffer limits
/// when the adapter has them.
fn wgpu_options() -> egui_wgpu::WgpuConfiguration {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends =
        wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY);
    let default_descriptor = Arc::clone(&setup.device_descriptor);
    setup.device_descriptor = Arc::new(move |adapter| {
        let mut descriptor = default_descriptor(adapter);
        descriptor.label = Some("fractals device");
        descriptor.required_features |= adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
        // Flame histograms are large storage buffers: allow what the GPU can.
        let supported = adapter.limits();
        descriptor.required_limits.max_storage_buffer_binding_size =
            supported.max_storage_buffer_binding_size;
        descriptor.required_limits.max_buffer_size = supported.max_buffer_size;
        descriptor
    });
    egui_wgpu::WgpuConfiguration {
        wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
        ..Default::default()
    }
}
