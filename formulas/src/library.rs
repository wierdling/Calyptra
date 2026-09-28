use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use scene::{DeMode, Fractal};

/// Formulas compiled into the binary.
const BUILTIN: &[(&str, &str)] = &[
    ("mandelbulb", include_str!("../wgsl/mandelbulb.wgsl")),
    ("mandelbox", include_str!("../wgsl/mandelbox.wgsl")),
    ("menger", include_str!("../wgsl/menger.wgsl")),
    (
        "kifs_octahedron",
        include_str!("../wgsl/kifs_octahedron.wgsl"),
    ),
    ("amazing_surf", include_str!("../wgsl/amazing_surf.wgsl")),
];

/// Where the built-in formula sources live in the source tree; debug builds
/// load from here so edits hot-reload.
const SOURCE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wgsl");

#[derive(Clone, Debug, PartialEq)]
pub struct ParamDef {
    pub name: String,
    pub default: f32,
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormulaDef {
    pub id: String,
    pub name: String,
    /// Never [`DeMode::Auto`].
    pub de_mode: DeMode,
    pub bailout: f32,
    pub params: Vec<ParamDef>,
    pub source: String,
}

/// Parses a formula file's metadata header.
pub fn parse_formula(id: &str, source: &str) -> Result<FormulaDef, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!(
            "invalid formula id {id:?}: use letters, digits and _"
        ));
    }
    let mut def = FormulaDef {
        id: id.to_owned(),
        name: id.to_owned(),
        de_mode: DeMode::Linear,
        bailout: 100.0,
        params: Vec::new(),
        source: source.to_owned(),
    };
    for (number, line) in source.lines().enumerate() {
        let Some(rest) = line.trim().strip_prefix("// @") else {
            continue;
        };
        let err = |msg: &str| format!("{id}.wgsl:{}: {msg}", number + 1);
        let (key, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let value = value.trim();
        match key {
            "name" => def.name = value.to_owned(),
            "de" => {
                def.de_mode = match value {
                    "logarithmic" => DeMode::Logarithmic,
                    "linear" => DeMode::Linear,
                    "box" => DeMode::Box,
                    _ => return Err(err("@de must be logarithmic, linear or box")),
                }
            }
            "bailout" => {
                def.bailout = value.parse().map_err(|_| err("@bailout needs a number"))?;
            }
            "param" => {
                let fields: Vec<&str> = value.split_whitespace().collect();
                let [name, default, min, max] = fields[..] else {
                    return Err(err("@param needs: name default min max"));
                };
                let number = |s: &str| {
                    s.parse::<f32>()
                        .map_err(|_| err("@param values must be numbers"))
                };
                def.params.push(ParamDef {
                    name: name.to_owned(),
                    default: number(default)?,
                    min: number(min)?,
                    max: number(max)?,
                });
            }
            _ => return Err(err(&format!("unknown metadata key @{key}"))),
        }
    }
    if def.params.len() > Fractal::MAX_PARAMS {
        return Err(format!("{id}: at most {} parameters", Fractal::MAX_PARAMS));
    }
    if !source.contains(&format!("fn {id}(")) {
        return Err(format!("{id}.wgsl must define `fn {id}(...)`"));
    }
    Ok(def)
}

/// The set of available formulas.
pub struct Library {
    formulas: BTreeMap<String, FormulaDef>,
    /// Directory to watch for changes, if loaded from disk.
    watch_dir: Option<PathBuf>,
    stamps: BTreeMap<PathBuf, SystemTime>,
    generation: u64,
}

impl Library {
    /// Debug builds load from the source tree (hot reload); release builds
    /// use the embedded copies.
    pub fn load_default() -> Result<Self, String> {
        let dir = Path::new(SOURCE_DIR);
        if cfg!(debug_assertions) && dir.is_dir() {
            Self::from_dir(dir)
        } else {
            Self::builtin()
        }
    }

    pub fn builtin() -> Result<Self, String> {
        let formulas = BUILTIN
            .iter()
            .map(|(id, source)| parse_formula(id, source).map(|def| (def.id.clone(), def)))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            formulas,
            watch_dir: None,
            stamps: BTreeMap::new(),
            generation: 0,
        })
    }

    pub fn from_dir(dir: &Path) -> Result<Self, String> {
        let mut library = Self {
            formulas: BTreeMap::new(),
            watch_dir: Some(dir.to_owned()),
            stamps: BTreeMap::new(),
            generation: 0,
        };
        library.load_dir()?;
        Ok(library)
    }

    fn load_dir(&mut self) -> Result<(), String> {
        let dir = self
            .watch_dir
            .clone()
            .expect("load_dir without a directory");
        let mut formulas = BTreeMap::new();
        let mut stamps = BTreeMap::new();
        for (path, stamp) in wgsl_files(&dir)? {
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let source =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let def = parse_formula(id, &source)?;
            formulas.insert(def.id.clone(), def);
            stamps.insert(path, stamp);
        }
        // Only replaced on success: a broken file keeps the last good set.
        self.stamps = stamps;
        self.formulas = formulas;
        self.generation += 1;
        Ok(())
    }

    /// Re-reads the directory if any file was added, removed or modified.
    /// Returns `None` when nothing changed.
    pub fn reload_if_changed(&mut self) -> Option<Result<(), String>> {
        let dir = self.watch_dir.as_ref()?;
        let current: BTreeMap<_, _> = wgsl_files(dir).ok()?.into_iter().collect();
        if current == self.stamps {
            return None;
        }
        let result = self.load_dir();
        if result.is_err() {
            // Remember the broken state so it is not re-parsed every poll.
            self.stamps = current;
        }
        Some(result)
    }

    /// Incremented on every (re)load; lets callers detect changes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn get(&self, id: &str) -> Option<&FormulaDef> {
        self.formulas.get(id)
    }

    /// All formulas, sorted by id.
    pub fn formulas(&self) -> impl Iterator<Item = &FormulaDef> {
        self.formulas.values()
    }

    /// Fills in missing parameters with defaults and drops extras, so every
    /// slot has exactly one value per declared parameter.
    pub fn normalize(&self, fractal: &mut Fractal) {
        for slot in &mut fractal.slots {
            if let Some(def) = self.get(&slot.formula) {
                slot.params.truncate(def.params.len());
                for param in &def.params[slot.params.len()..] {
                    slot.params.push(param.default);
                }
            }
            slot.repeat = slot.repeat.max(1);
        }
    }
}

fn wgsl_files(dir: &Path) -> Result<Vec<(PathBuf, SystemTime)>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "wgsl") {
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            files.push((path, modified));
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_formulas_parse() {
        let library = Library::builtin().unwrap();
        assert_eq!(library.formulas().count(), BUILTIN.len());
        let bulb = library.get("mandelbulb").unwrap();
        assert_eq!(bulb.de_mode, DeMode::Logarithmic);
        assert_eq!(bulb.params[0].name, "power");
        assert_eq!(bulb.params[0].default, 8.0);
    }

    #[test]
    fn source_dir_matches_embedded() {
        // Guards against adding a .wgsl file but forgetting BUILTIN.
        let from_disk = Library::from_dir(Path::new(SOURCE_DIR)).unwrap();
        let ids: Vec<_> = from_disk.formulas().map(|f| f.id.clone()).collect();
        let builtin = Library::builtin().unwrap();
        let builtin_ids: Vec<_> = builtin.formulas().map(|f| f.id.clone()).collect();
        assert_eq!(ids, builtin_ids);
    }

    #[test]
    fn bad_metadata_is_reported_with_line() {
        let err = parse_formula("x", "// @param a 1 2\nfn x() {}").unwrap_err();
        assert!(err.contains("x.wgsl:1"), "{err}");
    }

    #[test]
    fn missing_entry_point_is_reported() {
        let err = parse_formula("x", "fn y() {}").unwrap_err();
        assert!(err.contains("fn x("), "{err}");
    }

    #[test]
    fn normalize_fills_defaults() {
        let library = Library::builtin().unwrap();
        let mut fractal = Fractal {
            slots: vec![scene::FormulaSlot::new("mandelbox", vec![2.0])],
            ..Default::default()
        };
        library.normalize(&mut fractal);
        assert_eq!(fractal.slots[0].params, vec![2.0, 0.5, 1.0, 1.0]);
    }
}
