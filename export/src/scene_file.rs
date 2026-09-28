use std::path::Path;

use scene::Scene;
use serde::{Deserialize, Serialize};

use crate::ExportError;

/// PNG iTXt keyword holding the scene JSON.
pub(crate) const PNG_SCENE_KEY: &str = "fractals-scene";
const FORMAT: &str = "fractals-scene";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct SceneFile {
    format: String,
    version: u32,
    scene: Scene,
}

pub fn scene_to_json(scene: &Scene) -> Result<String, ExportError> {
    let file = SceneFile {
        format: FORMAT.to_owned(),
        version: VERSION,
        scene: scene.clone(),
    };
    serde_json::to_string_pretty(&file).map_err(|e| ExportError::Scene(e.to_string()))
}

fn scene_from_json(json: &str) -> Result<Scene, ExportError> {
    let file: SceneFile =
        serde_json::from_str(json).map_err(|e| ExportError::Scene(e.to_string()))?;
    if file.format != FORMAT {
        return Err(ExportError::Scene(format!(
            "not a scene file ({:?})",
            file.format
        )));
    }
    if file.version > VERSION {
        return Err(ExportError::Scene(format!(
            "written by a newer version (format {}); please update",
            file.version
        )));
    }
    Ok(file.scene)
}

pub fn save_scene(path: &Path, scene: &Scene) -> Result<(), ExportError> {
    std::fs::write(path, scene_to_json(scene)?)?;
    Ok(())
}

/// Loads a scene file, or the scene embedded in an exported PNG.
pub fn load_scene(path: &Path) -> Result<Scene, ExportError> {
    let is_png = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if !is_png {
        return scene_from_json(&std::fs::read_to_string(path)?);
    }
    let file = std::io::BufReader::new(std::fs::File::open(path)?);
    let reader = png::Decoder::new(file)
        .read_info()
        .map_err(|e| ExportError::Png(e.to_string()))?;
    let chunk = reader
        .info()
        .utf8_text
        .iter()
        .find(|chunk| chunk.keyword == PNG_SCENE_KEY)
        .ok_or_else(|| ExportError::Scene("this PNG has no embedded scene".into()))?;
    let json = chunk
        .get_text()
        .map_err(|e| ExportError::Png(e.to_string()))?;
    scene_from_json(&json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ImageFormat, save_image};

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("fractals-scene-test-{}-{name}", std::process::id()))
    }

    fn distinctive_scene() -> Scene {
        let mut scene = Scene::default();
        scene.fractal.iterations = 17;
        scene.coloring.offset = 0.375;
        scene.display.exposure_ev = -1.5;
        scene
    }

    #[test]
    fn json_round_trip() {
        let path = temp_path("s.json");
        save_scene(&path, &distinctive_scene()).unwrap();
        assert_eq!(load_scene(&path).unwrap(), distinctive_scene());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn scene_is_recovered_from_exported_png() {
        let path = temp_path("s.png");
        let image = render::HdrImage {
            width: 2,
            height: 2,
            pixels: vec![[0.5; 4]; 4],
        };
        save_image(&path, &image, ImageFormat::Png8, &distinctive_scene()).unwrap();
        assert_eq!(load_scene(&path).unwrap(), distinctive_scene());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_other_json() {
        let path = temp_path("other.json");
        std::fs::write(
            &path,
            r#"{"format": "something-else", "version": 1, "scene": {}}"#,
        )
        .unwrap();
        assert!(load_scene(&path).is_err());
        std::fs::remove_file(&path).ok();
    }
}
