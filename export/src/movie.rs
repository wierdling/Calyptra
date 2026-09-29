//! Movie rendering: animation → numbered 16-bit PNG frames → ffmpeg.
//!
//! Frames are written to a folder and skipped if already present, so an
//! interrupted render resumes where it stopped. A `job.json` in the folder
//! records what the frames belong to; a different animation or resolution
//! refuses to mix its frames into the same folder.

use std::path::{Path, PathBuf};
use std::process::Command;

use anim::Animation;
use render::{FlameRenderer, RaymarchRenderer, Renderer, StillSettings};
use scene::FractalKind;
use serde::{Deserialize, Serialize};

use crate::ExportError;
use crate::image::save_frame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCodec {
    /// Plays everywhere.
    H264,
    /// Half the size at the same quality; 10-bit.
    H265,
    /// Editing-friendly intermediate, 10-bit 4:2:2, large files.
    ProRes,
    /// Keep the PNG frames only.
    FramesOnly,
}

impl VideoCodec {
    pub const ALL: [Self; 4] = [Self::H264, Self::H265, Self::ProRes, Self::FramesOnly];

    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264 MP4 (plays everywhere)",
            Self::H265 => "H.265 MP4 (smaller, 10-bit)",
            Self::ProRes => "ProRes 422 HQ MOV (for editing)",
            Self::FramesOnly => "PNG frames only",
        }
    }

    pub fn extension(self) -> Option<&'static str> {
        match self {
            Self::H264 | Self::H265 => Some("mp4"),
            Self::ProRes => Some("mov"),
            Self::FramesOnly => None,
        }
    }

    fn ffmpeg_args(self) -> &'static [&'static str] {
        match self {
            Self::H264 => &[
                "-c:v",
                "libx264",
                "-preset",
                "slow",
                "-crf",
                "16",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ],
            Self::H265 => &[
                "-c:v",
                "libx265",
                "-preset",
                "slow",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p10le",
                "-tag:v",
                "hvc1",
                "-movflags",
                "+faststart",
            ],
            Self::ProRes => &[
                "-c:v",
                "prores_ks",
                "-profile:v",
                "3",
                "-pix_fmt",
                "yuv422p10le",
            ],
            Self::FramesOnly => &[],
        }
    }
}

#[derive(Clone, Debug)]
pub struct MovieSettings {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub codec: VideoCodec,
    /// Video file to write (ignored for [`VideoCodec::FramesOnly`]).
    pub output: PathBuf,
    pub frames_dir: PathBuf,
    /// ffmpeg executable; a bare `ffmpeg` is looked up on PATH.
    pub ffmpeg: PathBuf,
}

/// Progress reports. The callback returns `false` to cancel.
#[derive(Clone, Copy, Debug)]
pub enum MovieEvent {
    /// `frame` of `total` is done (`skipped` = already on disk).
    Frame {
        frame: u32,
        total: u32,
        skipped: bool,
    },
    /// Fraction of the current frame rendered.
    FrameProgress(f32),
    Encoding,
}

/// What the frames in a folder were rendered from.
#[derive(Serialize, Deserialize, PartialEq)]
struct Job {
    width: u32,
    height: u32,
    samples: u32,
    animation: Animation,
}

const JOB_FILE: &str = "job.json";

pub fn frame_path(frames_dir: &Path, frame: u32) -> PathBuf {
    frames_dir.join(format!("frame_{frame:06}.png"))
}

/// Renders every frame of `animation` (resuming if frames exist) and
/// encodes the movie. Blocking; run it on a worker thread.
pub fn render_movie(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    library: &formulas::Library,
    animation: &Animation,
    settings: &MovieSettings,
    mut progress: impl FnMut(MovieEvent) -> bool,
) -> Result<(), ExportError> {
    if animation.keyframes.is_empty() {
        return Err(ExportError::Movie("the animation has no keyframes".into()));
    }
    std::fs::create_dir_all(&settings.frames_dir)?;
    check_job(settings, animation)?;

    let total = animation.frame_count();
    let mut raymarch = RaymarchRenderer::new(device);
    // Created on first use: most movies are one kind or the other.
    let mut flame: Option<FlameRenderer> = None;
    let mut current_de: Option<String> = None;
    let still = StillSettings {
        width: settings.width,
        height: settings.height,
        samples: settings.samples,
    };

    for frame in 0..total {
        let path = frame_path(&settings.frames_dir, frame);
        if path.exists() {
            if !progress(MovieEvent::Frame {
                frame,
                total,
                skipped: true,
            }) {
                return Err(ExportError::Cancelled);
            }
            continue;
        }
        let mut scene = animation
            .scene_at(animation.frame_time(frame))
            .expect("animation has keyframes");
        let renderer: &mut dyn Renderer = match scene.kind {
            FractalKind::Flame => flame.get_or_insert_with(|| FlameRenderer::new(device)),
            FractalKind::Distance => {
                library.normalize(&mut scene.fractal);
                let de = formulas::compose(&scene.fractal, library)
                    .map_err(|e| ExportError::Movie(format!("frame {frame}: {e}")))?;
                if current_de.as_ref() != Some(&de) {
                    raymarch.set_de_source(device, de.clone());
                    if let Some(error) = raymarch.error() {
                        return Err(ExportError::Movie(format!("frame {frame}: {error}")));
                    }
                    current_de = Some(de);
                }
                &mut raymarch
            }
        };

        let mut cancelled = false;
        let image = render::render_still(device, queue, renderer, &scene, still, |f| {
            cancelled = !progress(MovieEvent::FrameProgress(f));
            !cancelled
        })
        .ok_or(ExportError::Cancelled)?;
        // Write-then-rename: a half-written frame never looks finished.
        let partial = path.with_extension("partial");
        save_frame(&partial, &image, &scene)?;
        std::fs::rename(&partial, &path)?;

        if !progress(MovieEvent::Frame {
            frame,
            total,
            skipped: false,
        }) {
            return Err(ExportError::Cancelled);
        }
    }

    if settings.codec == VideoCodec::FramesOnly {
        return Ok(());
    }
    progress(MovieEvent::Encoding);
    encode(settings, animation.fps)
}

fn check_job(settings: &MovieSettings, animation: &Animation) -> Result<(), ExportError> {
    let job = Job {
        width: settings.width,
        height: settings.height,
        samples: settings.samples,
        animation: animation.clone(),
    };
    let path = settings.frames_dir.join(JOB_FILE);
    let json = serde_json::to_string_pretty(&job).map_err(|e| ExportError::Movie(e.to_string()))?;
    match std::fs::read_to_string(&path) {
        Ok(existing) => {
            let same = serde_json::from_str::<Job>(&existing).is_ok_and(|j| j == job);
            if !same {
                return Err(ExportError::Movie(format!(
                    "{} holds frames from a different render (animation, size or samples changed). \
                     Choose another folder or delete it.",
                    settings.frames_dir.display()
                )));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // A fresh job; stray frames from elsewhere would be reused silently.
            if std::fs::read_dir(&settings.frames_dir)?.next().is_some() {
                return Err(ExportError::Movie(format!(
                    "{} is not empty. Choose an empty folder for the frames.",
                    settings.frames_dir.display()
                )));
            }
            std::fs::write(&path, json)?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn ffmpeg_command(ffmpeg: &Path) -> Command {
    let mut command = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Don't flash a console window from the GUI app.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn encode(settings: &MovieSettings, fps: f64) -> Result<(), ExportError> {
    let pattern = settings.frames_dir.join("frame_%06d.png");
    let output = ffmpeg_command(&settings.ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .args(["-framerate", &fps.to_string()])
        .arg("-i")
        .arg(&pattern)
        // Chroma subsampling needs even dimensions.
        .args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"])
        .args(settings.codec.ffmpeg_args())
        .arg(&settings.output)
        .output()
        .map_err(|e| {
            ExportError::Movie(format!(
                "could not run {}: {e}. Is ffmpeg installed and on PATH?",
                settings.ffmpeg.display()
            ))
        })?;
    if !output.status.success() {
        return Err(ExportError::Movie(format!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// The first line of `ffmpeg -version`, or `None` if it cannot be run.
pub fn ffmpeg_version(ffmpeg: &Path) -> Option<String> {
    let output = ffmpeg_command(ffmpeg).arg("-version").output().ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fractals-movie-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    fn settings(dir: &Path) -> MovieSettings {
        MovieSettings {
            width: 64,
            height: 36,
            samples: 1,
            codec: VideoCodec::FramesOnly,
            output: dir.join("out.mp4"),
            frames_dir: dir.to_owned(),
            ffmpeg: "ffmpeg".into(),
        }
    }

    fn animation() -> Animation {
        let mut animation = Animation::default();
        animation.set_key(0.0, scene::Scene::default());
        animation
    }

    #[test]
    fn job_file_allows_resume_and_blocks_mixing() {
        let dir = temp_dir("job");
        std::fs::create_dir_all(&dir).unwrap();
        let settings = settings(&dir);
        check_job(&settings, &animation()).unwrap();
        // Same job again: resume is fine.
        check_job(&settings, &animation()).unwrap();
        // Different resolution: refused.
        let other = MovieSettings {
            width: 128,
            ..settings.clone()
        };
        assert!(check_job(&other, &animation()).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refuses_a_non_empty_folder_without_a_job() {
        let dir = temp_dir("stray");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("frame_000000.png"), b"not ours").unwrap();
        assert!(check_job(&settings(&dir), &animation()).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn frame_names_sort_in_order() {
        let dir = Path::new("frames");
        assert_eq!(frame_path(dir, 7), dir.join("frame_000007.png"));
    }
}
