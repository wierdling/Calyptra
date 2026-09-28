use std::path::PathBuf;
use std::time::SystemTime;

/// A shader source that is embedded in the binary but, in debug builds, read
/// from the source tree instead and re-read when it changes on disk.
pub(crate) struct WatchedFile {
    path: Option<PathBuf>,
    modified: Option<SystemTime>,
    contents: String,
}

impl WatchedFile {
    /// `path` is the file in the source tree; `embedded` its compiled-in copy.
    pub fn new(path: &str, embedded: &'static str) -> Self {
        let path = PathBuf::from(path);
        let mut file = Self {
            path: (cfg!(debug_assertions) && path.is_file()).then_some(path),
            modified: None,
            contents: embedded.to_owned(),
        };
        file.changed();
        file
    }

    /// Re-reads the file if its modification time changed.
    pub fn changed(&mut self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if modified == self.modified {
            return false;
        }
        self.modified = modified;
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                self.contents = contents;
                true
            }
            // Editors sometimes truncate-then-write; try again next poll.
            Err(_) => {
                self.modified = None;
                false
            }
        }
    }

    pub fn contents(&self) -> &str {
        &self.contents
    }
}
