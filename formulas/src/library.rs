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
    (
        "quaternion_julia",
        include_str!("../wgsl/quaternion_julia.wgsl"),
    ),
    (
        "pseudo_kleinian",
        include_str!("../wgsl/pseudo_kleinian.wgsl"),
    ),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    BuiltIn,
    User,
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
    /// The iterated point is 4D (`ptr<function, vec4<f32>>`) instead of 3D.
    pub vec4_state: bool,
    /// Parameter index that sets the initial 4th coordinate (the 3D slice).
    pub init_w: Option<usize>,
    /// Defines `fn <id>_de(z: vec4<f32>, dr: f32, P: <id>_Params) -> f32`.
    pub custom_de: bool,
    pub origin: Origin,
    /// The file it was loaded from, if any.
    pub path: Option<PathBuf>,
}

/// Parses a formula file's metadata header.
pub fn parse_formula(id: &str, source: &str) -> Result<FormulaDef, String> {
    if id.is_empty()
        || id.starts_with(|c: char| c.is_ascii_digit())
        || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(format!(
            "invalid formula id {id:?}: the file name must use letters, digits and _, not starting with a digit"
        ));
    }
    let mut def = FormulaDef {
        id: id.to_owned(),
        name: id.to_owned(),
        de_mode: DeMode::Linear,
        bailout: 100.0,
        params: Vec::new(),
        source: source.to_owned(),
        vec4_state: false,
        init_w: None,
        custom_de: false,
        origin: Origin::BuiltIn,
        path: None,
    };
    let mut init_w_name = None;
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
                    "custom" => DeMode::Custom,
                    _ => return Err(err("@de must be logarithmic, linear, box or custom")),
                }
            }
            "bailout" => {
                def.bailout = value.parse().map_err(|_| err("@bailout needs a number"))?;
            }
            "state" => {
                def.vec4_state = match value {
                    "vec3" => false,
                    "vec4" => true,
                    _ => return Err(err("@state must be vec3 or vec4")),
                }
            }
            "init_w" => init_w_name = Some((value.to_owned(), number + 1)),
            "param" => {
                let fields: Vec<&str> = value.split_whitespace().collect();
                let [name, default, min, max] = fields[..] else {
                    return Err(err("@param needs: name default min max"));
                };
                if def.params.iter().any(|p| p.name == name) {
                    return Err(err(&format!("duplicate parameter {name:?}")));
                }
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
    if let Some((name, line)) = init_w_name {
        def.init_w = Some(
            def.params
                .iter()
                .position(|p| p.name == name)
                .ok_or_else(|| {
                    format!("{id}.wgsl:{line}: @init_w names unknown parameter {name:?}")
                })?,
        );
        if !def.vec4_state {
            return Err(format!("{id}.wgsl:{line}: @init_w needs `// @state vec4`"));
        }
    }
    if !source.contains(&format!("fn {id}(")) {
        return Err(format!("{id}.wgsl must define `fn {id}(...)`"));
    }
    def.custom_de = source.contains(&format!("fn {id}_de("));
    if def.de_mode == DeMode::Custom && !def.custom_de {
        return Err(format!(
            "{id}.wgsl: `@de custom` needs `fn {id}_de(z: vec4<f32>, dr: f32, P: {id}_Params) -> f32`"
        ));
    }
    Ok(def)
}

/// A directory of `.wgsl` formula files.
struct Folder {
    path: PathBuf,
    origin: Origin,
}

/// The set of available formulas: built-ins plus an optional user folder,
/// both hot-reloadable.
pub struct Library {
    formulas: BTreeMap<String, FormulaDef>,
    /// Use the compiled-in built-ins (else they come from a folder).
    embedded: bool,
    folders: Vec<Folder>,
    stamps: BTreeMap<PathBuf, SystemTime>,
    /// Problems from the last load; the affected files are skipped.
    errors: Vec<String>,
    generation: u64,
}

impl Library {
    /// Debug builds load built-ins from the source tree (hot reload);
    /// release builds use the embedded copies.
    pub fn load_default() -> Result<Self, String> {
        let dir = Path::new(SOURCE_DIR);
        if cfg!(debug_assertions) && dir.is_dir() {
            Self::from_dir(dir)
        } else {
            Self::builtin()
        }
    }

    pub fn builtin() -> Result<Self, String> {
        let mut library = Self::empty(true);
        library.load_all();
        library.check_builtins()?;
        Ok(library)
    }

    pub fn from_dir(dir: &Path) -> Result<Self, String> {
        let mut library = Self::empty(false);
        library.folders.push(Folder {
            path: dir.to_owned(),
            origin: Origin::BuiltIn,
        });
        library.load_all();
        library.check_builtins()?;
        Ok(library)
    }

    fn empty(embedded: bool) -> Self {
        Self {
            formulas: BTreeMap::new(),
            embedded,
            folders: Vec::new(),
            stamps: BTreeMap::new(),
            errors: Vec::new(),
            generation: 0,
        }
    }

    /// Built-ins must always load; a broken one is a bug, not a user error.
    fn check_builtins(&self) -> Result<(), String> {
        match self.errors.first() {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    /// Adds a folder of user formulas (created if missing). Errors in those
    /// files are reported by [`Self::errors`], never fatal.
    pub fn with_user_dir(mut self, dir: &Path) -> Self {
        if let Err(e) = std::fs::create_dir_all(dir) {
            self.errors.push(format!("{}: {e}", dir.display()));
        }
        self.folders.push(Folder {
            path: dir.to_owned(),
            origin: Origin::User,
        });
        self.load_all();
        self
    }

    /// The user formula folder, if one was added.
    pub fn user_dir(&self) -> Option<&Path> {
        self.folders
            .iter()
            .find(|f| f.origin == Origin::User)
            .map(|f| f.path.as_path())
    }

    fn load_all(&mut self) {
        let mut formulas = BTreeMap::new();
        let mut errors = Vec::new();
        if self.embedded {
            for (id, source) in BUILTIN {
                match parse_formula(id, source) {
                    Ok(def) => {
                        formulas.insert(def.id.clone(), def);
                    }
                    Err(e) => errors.push(e),
                }
            }
        }
        let mut stamps = BTreeMap::new();
        for folder in &self.folders {
            let files = match wgsl_files(&folder.path) {
                Ok(files) => files,
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            };
            for (path, stamp) in files {
                stamps.insert(path.clone(), stamp);
                let id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                let source = match std::fs::read_to_string(&path) {
                    Ok(source) => source,
                    Err(e) => {
                        errors.push(format!("{}: {e}", path.display()));
                        continue;
                    }
                };
                match parse_formula(id, &source) {
                    Ok(def) if formulas.contains_key(&def.id) => {
                        errors.push(format!(
                            "{}: a formula named {:?} already exists; rename the file",
                            path.display(),
                            def.id
                        ));
                    }
                    Ok(mut def) => {
                        def.origin = folder.origin;
                        def.path = Some(path.clone());
                        formulas.insert(def.id.clone(), def);
                    }
                    Err(e) => errors.push(e),
                }
            }
        }
        self.formulas = formulas;
        self.errors = errors;
        self.stamps = stamps;
        self.generation += 1;
    }

    /// Re-reads the folders if any file was added, removed or modified.
    /// Returns `true` if the library was reloaded.
    pub fn reload_if_changed(&mut self) -> bool {
        let mut current = BTreeMap::new();
        for folder in &self.folders {
            if let Ok(files) = wgsl_files(&folder.path) {
                current.extend(files);
            }
        }
        if current == self.stamps {
            return false;
        }
        self.load_all();
        true
    }

    /// Problems found by the last load (bad metadata, name clashes, I/O).
    pub fn errors(&self) -> &[String] {
        &self.errors
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

/// A commented starting point for a new user formula with id `id`.
pub fn formula_template(id: &str) -> String {
    format!(
        r#"// @name {id}
// @de linear
// @bailout 100
// @param scale 2 -3 3
// @param fold 1 0 2
//
// One iteration of the fractal. Save this file and the view updates.
//
//   z  - the point being iterated (add `// @state vec4` for a 4D point)
//   dr - running derivative, used for the distance estimate
//   c  - constant to add: the sample position, or the Julia constant
//   P  - parameters declared with @param above: P.scale, P.fold
//
// Header keys (each on its own comment line, like the ones above):
//   name, de (logarithmic | linear | box | custom), bailout,
//   param NAME DEFAULT MIN MAX (up to 8), state (vec3 | vec4),
//   init_w PARAM (starting 4th coordinate for vec4 formulas).
// With de custom, also define
//   fn {id}_de(z: vec4<f32>, dr: f32, P: {id}_Params) -> f32
// Prefix any helper functions with `{id}_` so hybrids don't clash.

fn {id}(z: ptr<function, vec3<f32>>, dr: ptr<function, f32>, c: vec3<f32>, P: {id}_Params) {{
    // Box fold, then scale and translate (a Mandelbox without the sphere fold).
    let v = clamp(*z, vec3<f32>(-P.fold), vec3<f32>(P.fold)) * 2.0 - *z;
    *z = v * P.scale + c;
    *dr = (*dr) * abs(P.scale) + 1.0;
}}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("calyptra-lib-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn builtin_formulas_parse() {
        let library = Library::builtin().unwrap();
        assert_eq!(library.formulas().count(), BUILTIN.len());
        let bulb = library.get("mandelbulb").unwrap();
        assert_eq!(bulb.de_mode, DeMode::Logarithmic);
        assert_eq!(bulb.params[0].name, "power");
        assert_eq!(bulb.params[0].default, 8.0);
        let quaternion = library.get("quaternion_julia").unwrap();
        assert!(quaternion.vec4_state && quaternion.init_w.is_some());
        assert!(library.get("pseudo_kleinian").unwrap().custom_de);
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
    fn custom_de_requires_its_function() {
        let err = parse_formula("x", "// @de custom\nfn x() {}").unwrap_err();
        assert!(err.contains("fn x_de("), "{err}");
    }

    #[test]
    fn init_w_requires_vec4_and_a_known_param() {
        let src = "// @param s 0 -1 1\n// @init_w s\nfn x() {}";
        assert!(parse_formula("x", src).unwrap_err().contains("@state vec4"));
        let src = "// @state vec4\n// @init_w nope\nfn x() {}";
        assert!(
            parse_formula("x", src)
                .unwrap_err()
                .contains("unknown parameter")
        );
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

    #[test]
    fn template_parses() {
        let def = parse_formula("my_formula", &formula_template("my_formula")).unwrap();
        assert_eq!(def.params.len(), 2);
    }

    #[test]
    fn user_folder_loads_skips_broken_files_and_reloads() {
        let dir = temp_dir("user");
        let library = Library::builtin().unwrap().with_user_dir(&dir);
        assert!(library.errors().is_empty());

        std::fs::write(dir.join("my_box.wgsl"), formula_template("my_box")).unwrap();
        std::fs::write(dir.join("broken.wgsl"), "// @de sideways\nfn broken() {}").unwrap();
        std::fs::write(dir.join("mandelbulb.wgsl"), formula_template("mandelbulb")).unwrap();
        let mut library = library;
        assert!(library.reload_if_changed());
        let user = library.get("my_box").unwrap();
        assert_eq!(user.origin, Origin::User);
        assert_eq!(user.params.len(), 2);
        // The broken file and the name clash are reported, not fatal.
        assert_eq!(library.errors().len(), 2, "{:?}", library.errors());
        assert_eq!(library.get("mandelbulb").unwrap().origin, Origin::BuiltIn);
        assert!(!library.reload_if_changed());
        std::fs::remove_dir_all(&dir).ok();
    }
}
