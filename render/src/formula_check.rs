//! Compiles one formula on its own and reports errors against its file, so
//! a user sees `my_formula.wgsl:6:13: …` instead of a line number in the
//! generated shader.

use scene::{FormulaSlot, Fractal};
use wgpu::naga;

use crate::FULLSCREEN_WGSL;

const RAYMARCH_WGSL: &str = include_str!("shaders/raymarch.wgsl");

/// Checks that formula `id` compiles inside the raymarch shader.
pub fn check_formula(library: &formulas::Library, id: &str) -> Result<(), String> {
    let def = library
        .get(id)
        .ok_or_else(|| format!("unknown formula {id:?}"))?;
    let mut fractal = Fractal {
        slots: vec![FormulaSlot::new(id, vec![])],
        ..Default::default()
    };
    library.normalize(&mut fractal);
    let de = formulas::compose(&fractal, library).map_err(|e| format!("{id}: {e}"))?;
    let source = format!("{FULLSCREEN_WGSL}\n{RAYMARCH_WGSL}\n{de}");

    let file = def.path.as_ref().and_then(|p| p.file_name()).map_or_else(
        || format!("{id}.wgsl"),
        |n| n.to_string_lossy().into_owned(),
    );
    // Lines before the formula's own text in the generated source.
    let offset = source
        .find(def.source.trim_end())
        .map_or(0, |start| source[..start].matches('\n').count() as u32);
    let file_lines = def.source.lines().count() as u32;
    let locate = |location: Option<naga::SourceLocation>| match location {
        Some(loc) if loc.line_number > offset && loc.line_number <= offset + file_lines => {
            format!("{file}:{}:{}", loc.line_number - offset, loc.line_position)
        }
        // Outside the file: usually a wrong signature seen from the caller.
        Some(loc) => format!("{file} (generated code line {})", loc.line_number),
        None => file.clone(),
    };

    let module = naga::front::wgsl::parse_str(&source)
        .map_err(|e| format!("{}: {}", locate(e.location(&source)), e.message()))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|e| format!("{}: {}", locate(e.location(&source)), error_chain(&e)))?;
    Ok(())
}

/// An error and its causes, joined: naga nests the useful detail.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_library(name: &str, source: &str) -> (formulas::Library, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("fractals-check-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.wgsl")), source).unwrap();
        (
            formulas::Library::builtin().unwrap().with_user_dir(&dir),
            dir,
        )
    }

    #[test]
    fn builtins_pass() {
        let library = formulas::Library::builtin().unwrap();
        for def in library.formulas() {
            check_formula(&library, &def.id).unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn template_passes() {
        let (library, dir) = user_library("fresh", &formulas::formula_template("fresh"));
        check_formula(&library, "fresh").unwrap();
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn errors_point_at_the_users_file_and_line() {
        let source = formulas::formula_template("typo").replace("abs(P.scale)", "abss(P.scale)");
        let line = source.lines().position(|l| l.contains("abss")).unwrap() + 1;
        let (library, dir) = user_library("typo", &source);
        let error = check_formula(&library, "typo").unwrap_err();
        assert!(error.starts_with(&format!("typo.wgsl:{line}:")), "{error}");
        assert!(error.contains("abss"), "{error}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn type_errors_are_reported_too() {
        let source = formulas::formula_template("types")
            .replace("*dr = (*dr) * abs(P.scale) + 1.0;", "*dr = vec2<f32>(1.0);");
        let (library, dir) = user_library("types", &source);
        let error = check_formula(&library, "types").unwrap_err();
        assert!(error.starts_with("types.wgsl:"), "{error}");
        std::fs::remove_dir_all(dir).ok();
    }
}
