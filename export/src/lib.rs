//! Writing finished images and reading/writing scene files.

mod image;
mod movie;
mod scene_file;

pub use image::{ImageFormat, save_image};
pub use movie::{MovieEvent, MovieSettings, VideoCodec, ffmpeg_version, render_movie};
pub use scene_file::{Project, load_project, load_scene, save_scene, scene_to_json};

#[derive(Debug)]
pub enum ExportError {
    Io(std::io::Error),
    Png(String),
    Exr(String),
    Scene(String),
    Movie(String),
    Cancelled,
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Png(e) => write!(f, "PNG: {e}"),
            Self::Exr(e) => write!(f, "EXR: {e}"),
            Self::Scene(e) => write!(f, "scene file: {e}"),
            Self::Movie(e) => write!(f, "{e}"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
