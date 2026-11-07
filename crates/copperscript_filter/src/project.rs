use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

const DEFAULT_DATA_PATH: &str = "./packs/data";

/// Reads the data folder from the project's `config.json`.
pub fn data_dir(root: &Path) -> Result<PathBuf> {
    let path = root.join("config.json");
    let text =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let config: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    // Skip `.` components, so paths in error messages read `<root>/packs/data`.
    let relative: PathBuf = Path::new(data_path(&config))
        .components()
        .filter(|component| *component != Component::CurDir)
        .collect();
    Ok(root.join(relative))
}

fn data_path(config: &Value) -> &str {
    config["regolith"]["dataPath"]
        .as_str()
        .unwrap_or(DEFAULT_DATA_PATH)
}

/// The target directory to build in, if cargo's own choice is inside the data folder.
///
/// Regolith copies the data folder into `.regolith/tmp` on every run and watches it for changes,
/// so a `target/` there would be copied each time and would retrigger `regolith watch`.
pub fn target_dir_override(root: &Path, data_dir: &Path, target_dir: &Path) -> Option<PathBuf> {
    normalize(target_dir)
        .starts_with(normalize(data_dir))
        .then(|| root.join(".regolith").join("copperscript-target"))
}

/// Resolves `.` and `..` in `path`, then symlinks in the part that exists, so paths from cargo and
/// from Regolith compare equal.
fn normalize(path: &Path) -> PathBuf {
    // Resolved first, because `Path::file_name` is `None` for a path ending in `..`.
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }

    let mut existing = clean.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(mut canonical) = existing.canonicalize() {
            canonical.extend(rest.iter().rev());
            return canonical;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                existing = parent;
            }
            _ => return clean,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn data_path_from_config() {
        let config = json!({ "regolith": { "dataPath": "./data" } });
        assert_eq!(data_path(&config), "./data");
    }

    #[test]
    fn data_path_default() {
        assert_eq!(data_path(&json!({ "regolith": {} })), "./packs/data");
        assert_eq!(data_path(&json!({})), "./packs/data");
    }

    #[test]
    fn target_inside_data_is_moved() {
        let root = Path::new("/nonexistent/project");
        let data = root.join("./packs/data");
        let target = root.join("packs/data/copperscript/target");
        assert_eq!(
            target_dir_override(root, &data, &target),
            Some(root.join(".regolith/copperscript-target"))
        );
    }

    #[test]
    fn workspace_target_is_kept() {
        let root = Path::new("/nonexistent/repo/examples/hello-world");
        let data = root.join("./packs/data");
        let target = Path::new("/nonexistent/repo/target");
        assert_eq!(target_dir_override(root, &data, target), None);
    }

    #[test]
    fn sibling_with_common_prefix_is_kept() {
        let root = Path::new("/nonexistent/project");
        let data = root.join("packs/data");
        let target = root.join("packs/data-target");
        assert_eq!(target_dir_override(root, &data, &target), None);
    }

    #[test]
    fn normalize_resolves_dots() {
        // Compared with another normalized path, because the existing root canonicalizes
        // differently per platform, such as `\\?\C:\` on Windows.
        assert_eq!(
            normalize(Path::new("/nonexistent/a/./b/../c")),
            normalize(Path::new("/nonexistent/a/c"))
        );
    }

    #[test]
    fn normalize_canonicalizes_after_parent_dir() {
        let temp = std::env::temp_dir();
        assert_eq!(
            normalize(&temp.join("nonexistent/..")),
            temp.canonicalize().unwrap()
        );
    }
}
