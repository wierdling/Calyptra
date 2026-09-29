use std::path::Path;

use anim::Animation;
use scene::Scene;
use serde::{Deserialize, Serialize};

use crate::ExportError;

/// PNG iTXt keyword holding the scene JSON.
pub(crate) const PNG_SCENE_KEY: &str = "calyptra-scene";
const FORMAT: &str = "calyptra-scene";
/// What the format was called before the program was renamed Calyptra;
/// still read, as both the format tag and the PNG keyword.
const LEGACY_FORMAT: &str = "fractals-scene";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct SceneFile {
    format: String,
    version: u32,
    scene: Scene,
    /// Optional, so files without it (and older readers) keep working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    animation: Option<Animation>,
}

/// A scene plus its animation, if any: what File → Open/Save handle.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub scene: Scene,
    pub animation: Option<Animation>,
}

fn to_json(scene: &Scene, animation: Option<&Animation>) -> Result<String, ExportError> {
    let file = SceneFile {
        format: FORMAT.to_owned(),
        version: VERSION,
        scene: scene.clone(),
        animation: animation.cloned(),
    };
    serde_json::to_string_pretty(&file).map_err(|e| ExportError::Scene(e.to_string()))
}

pub fn scene_to_json(scene: &Scene) -> Result<String, ExportError> {
    to_json(scene, None)
}

fn project_from_json(json: &str) -> Result<Project, ExportError> {
    let file: SceneFile =
        serde_json::from_str(json).map_err(|e| ExportError::Scene(e.to_string()))?;
    if file.format != FORMAT && file.format != LEGACY_FORMAT {
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
    Ok(Project {
        scene: file.scene,
        animation: file.animation,
    })
}

pub fn save_scene(
    path: &Path,
    scene: &Scene,
    animation: Option<&Animation>,
) -> Result<(), ExportError> {
    std::fs::write(path, to_json(scene, animation)?)?;
    Ok(())
}

/// Loads a scene file's scene (ignoring any animation).
pub fn load_scene(path: &Path) -> Result<Scene, ExportError> {
    load_project(path).map(|project| project.scene)
}

/// Loads a scene file, or the scene embedded in an exported PNG.
pub fn load_project(path: &Path) -> Result<Project, ExportError> {
    let is_png = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if !is_png {
        return project_from_json(&std::fs::read_to_string(path)?);
    }
    let file = std::io::BufReader::new(std::fs::File::open(path)?);
    let reader = png::Decoder::new(file)
        .read_info()
        .map_err(|e| ExportError::Png(e.to_string()))?;
    let chunk = reader
        .info()
        .utf8_text
        .iter()
        .find(|chunk| chunk.keyword == PNG_SCENE_KEY || chunk.keyword == LEGACY_FORMAT)
        .ok_or_else(|| ExportError::Scene("this PNG has no embedded scene".into()))?;
    let json = chunk
        .get_text()
        .map_err(|e| ExportError::Png(e.to_string()))?;
    project_from_json(&json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ImageFormat, save_image};

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("calyptra-scene-test-{}-{name}", std::process::id()))
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
        save_scene(&path, &distinctive_scene(), None).unwrap();
        assert_eq!(load_scene(&path).unwrap(), distinctive_scene());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn scene_files_from_before_the_rename_still_load() {
        let json = scene_to_json(&distinctive_scene())
            .unwrap()
            .replace(FORMAT, LEGACY_FORMAT);
        assert_eq!(project_from_json(&json).unwrap().scene, distinctive_scene());
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
    fn animation_round_trips_with_the_scene() {
        let path = temp_path("anim.json");
        let mut animation = Animation::default();
        animation.set_key(0.0, Scene::default());
        animation.set_key(2.5, distinctive_scene());
        save_scene(&path, &distinctive_scene(), Some(&animation)).unwrap();
        let project = load_project(&path).unwrap();
        assert_eq!(project.animation, Some(animation));
        assert_eq!(project.scene, distinctive_scene());
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
