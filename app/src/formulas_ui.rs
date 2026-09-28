//! Formulas panel: the user formula folder, creating and editing formulas,
//! and their compile problems.

use std::path::{Path, PathBuf};

use formulas::{Library, Origin};

/// Where user formulas live: `%APPDATA%\Fractals\formulas` on Windows.
pub fn user_formula_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    base.join("Fractals").join("formulas")
}

/// Every problem worth showing: load errors plus user formulas that do not
/// compile. Built-in formulas are covered by tests.
pub fn formula_problems(library: &Library) -> Vec<String> {
    let mut problems = library.errors().to_vec();
    for def in library.formulas().filter(|d| d.origin == Origin::User) {
        if let Err(error) = render::check_formula(library, &def.id) {
            problems.push(error);
        }
    }
    problems
}

#[derive(Default)]
pub struct FormulasPanel {
    new_name: String,
    message: Option<Result<String, String>>,
}

impl FormulasPanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, library: &Library, problems: &[String]) {
        let Some(dir) = library.user_dir() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.small(dir.display().to_string());
            if ui.small_button("Open folder").clicked() {
                open(dir);
            }
        });

        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_name)
                    .hint_text("new_formula_name")
                    .desired_width(150.0),
            );
            if ui
                .button("New formula")
                .on_hover_text("Create a documented starter file and open it in your editor")
                .clicked()
            {
                self.message = Some(create(dir, &self.new_name, library));
                if matches!(self.message, Some(Ok(_))) {
                    self.new_name.clear();
                }
            }
        });
        match &self.message {
            Some(Ok(message)) => {
                ui.small(message);
            }
            Some(Err(message)) => {
                ui.colored_label(ui.visuals().error_fg_color, message);
            }
            None => {}
        }

        let user: Vec<_> = library
            .formulas()
            .filter(|d| d.origin == Origin::User)
            .collect();
        if user.is_empty() {
            ui.weak("No custom formulas yet.");
        }
        for def in user {
            ui.horizontal(|ui| {
                ui.label(&def.name);
                ui.weak(format!("({})", def.id));
                if let Some(path) = &def.path
                    && ui.small_button("Edit").clicked()
                {
                    open(path);
                }
            });
        }

        let error_color = ui.visuals().error_fg_color;
        for problem in problems {
            ui.label(egui::RichText::new(problem).monospace().color(error_color));
        }
        if !problems.is_empty() {
            ui.weak("Fix and save the file: it reloads automatically.");
        }
    }
}

/// Creates `<dir>/<id>.wgsl` from the template and opens it.
fn create(dir: &Path, name: &str, library: &Library) -> Result<String, String> {
    let id: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if id.is_empty() || id.starts_with(|c: char| c.is_ascii_digit()) {
        return Err("Enter a name starting with a letter".into());
    }
    if library.get(&id).is_some() {
        return Err(format!("A formula named {id:?} already exists"));
    }
    let path = dir.join(format!("{id}.wgsl"));
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    std::fs::write(&path, formulas::formula_template(&id)).map_err(|e| e.to_string())?;
    open(&path);
    Ok(format!("Created {id}.wgsl: pick it in a formula slot"))
}

/// Opens a file or folder with its default Windows application.
fn open(path: &Path) {
    if let Err(error) = std::process::Command::new("explorer").arg(path).spawn() {
        log::error!("opening {}: {error}", path.display());
    }
}
