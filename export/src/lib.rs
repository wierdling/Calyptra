//! Writing finished images and reading/writing scene files.

mod image;
mod scene_file;

pub use image::{ImageFormat, save_image};
pub use scene_file::{load_scene, save_scene, scene_to_json};

#[derive(Debug)]
pub enum ExportError {
    Io(std::io::Error),
    Png(String),
    Exr(String),
    Scene(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Png(e) => write!(f, "PNG: {e}"),
            Self::Exr(e) => write!(f, "EXR: {e}"),
            Self::Scene(e) => write!(f, "scene file: {e}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
