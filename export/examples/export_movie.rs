//! Command-line movie render, the same path as the app's Render movie window.
//!
//!     cargo run --release -p export --example export_movie -- OUT.mp4 [WIDTH HEIGHT SAMPLES] [--project SCENE.json]
//!
//! Without `--project`, renders a demo: a half orbit around the Mandelbulb
//! while the power morphs from 8 to 5. Frames go to OUT_frames/ and are
//! reused if the render is interrupted and restarted.

use std::path::PathBuf;
use std::time::Instant;

use anim::{Animation, Easing};
use export::{MovieEvent, MovieSettings, VideoCodec};
use glam::DVec3;
use scene::{Camera, Scene};

fn demo_animation() -> Animation {
    let mut animation = Animation {
        duration: 4.0,
        fps: 30.0,
        ..Default::default()
    };
    for (time, angle, power) in [(0.0, 0.0f64, 8.0f32), (2.0, 90.0, 6.5), (4.0, 180.0, 5.0)] {
        let mut scene = Scene::default();
        let a = angle.to_radians();
        scene.camera = Camera::looking_at(
            DVec3::new(2.6 * a.sin(), 0.9, 2.6 * a.cos()),
            DVec3::ZERO,
            50.0,
        );
        scene.fractal.slots[0].params = vec![power];
        animation.set_key(time, scene);
    }
    animation.keyframes[0].easing = Easing::EaseIn;
    animation.keyframes[1].easing = Easing::EaseOut;
    animation
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let project = args.iter().position(|a| a == "--project").map(|i| {
        args.remove(i);
        PathBuf::from(args.remove(i))
    });
    let output = PathBuf::from(
        args.first()
            .ok_or("usage: OUT.mp4 [WIDTH HEIGHT SAMPLES]")?,
    );
    let number = |i: usize, default: u32| args.get(i).map_or(Ok(default), |a| a.parse());
    let animation = match &project {
        Some(path) => export::load_project(path)?
            .animation
            .ok_or("the project has no animation")?,
        None => demo_animation(),
    };
    let codec = match output.extension().and_then(|e| e.to_str()) {
        Some("mov") => VideoCodec::ProRes,
        _ => VideoCodec::H264,
    };
    let settings = MovieSettings {
        width: number(1, 640)?,
        height: number(2, 360)?,
        samples: number(3, 4)?,
        codec,
        frames_dir: output.with_file_name(format!(
            "{}_frames",
            output.file_stem().unwrap_or_default().to_string_lossy()
        )),
        output: output.clone(),
        ffmpeg: "ffmpeg".into(),
    };
    println!("ffmpeg: {:?}", export::ffmpeg_version(&settings.ffmpeg));

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let library = formulas::Library::builtin()?;

    let start = Instant::now();
    let (mut rendered, mut skipped) = (0, 0);
    export::render_movie(&device, &queue, &library, &animation, &settings, |event| {
        match event {
            MovieEvent::Frame { skipped: true, .. } => skipped += 1,
            MovieEvent::Frame { frame, total, .. } => {
                rendered += 1;
                if frame % 10 == 0 || frame + 1 == total {
                    eprintln!("frame {}/{total}", frame + 1);
                }
            }
            MovieEvent::Encoding => eprintln!("encoding..."),
            MovieEvent::FrameProgress(_) => {}
        }
        true
    })?;
    println!(
        "{rendered} frames rendered, {skipped} reused, {} in {:.1?}",
        output.display(),
        start.elapsed()
    );
    Ok(())
}
