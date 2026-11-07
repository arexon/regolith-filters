use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::manifest::ScriptModule;

/// The plugin's DLL is loaded by the Windows build of the game.
pub const TARGET: &str = "x86_64-pc-windows-msvc";

/// The parts of `cargo metadata` the filter uses.
#[derive(Debug, PartialEq, Eq)]
pub struct Plugin {
    /// The package ID, which cargo's build messages refer to.
    pub id: String,
    pub name: String,
    /// The name of the cdylib target, which names the DLL.
    pub lib_name: String,
    pub target_dir: PathBuf,
    /// The script modules that the enabled features of `copperscript` bind.
    pub modules: Vec<ScriptModule>,
}

/// A cargo command for the crate at `manifest`, run from the crate's folder.
///
/// Cargo and rustup find `.cargo/config.toml` and `rust-toolchain.toml` from the working directory.
/// Regolith's, `.regolith/tmp`, would hide those in the crate's folder, such as the Windows linker
/// a crate sets for cross builds.
fn cargo(manifest: &Path) -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    if let Some(dir) = manifest.parent() {
        command.current_dir(dir);
    }
    command
}

/// Runs `cargo metadata` on the plugin crate.
pub fn metadata(manifest: &Path) -> Result<Plugin> {
    let output = cargo(manifest)
        .args([
            "metadata",
            "--quiet",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest)
        .stderr(Stdio::inherit())
        .output()
        .context("cannot run cargo, is it on the PATH?")?;
    if !output.status.success() {
        bail!("cargo metadata failed for {}", manifest.display());
    }
    let metadata: Value =
        serde_json::from_slice(&output.stdout).context("cannot read cargo metadata's output")?;
    plugin(&metadata)
}

fn plugin(metadata: &Value) -> Result<Plugin> {
    let packages = metadata["packages"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let package = |id: &str| packages.iter().find(|package| package["id"] == id);
    let node = |id: &str| nodes.iter().find(|node| node["id"] == id);

    // `resolve.root` is the package of the manifest passed to cargo, even inside a workspace.
    let id = metadata["resolve"]["root"]
        .as_str()
        .context("the crate's Cargo.toml has no [package]")?;
    let root = package(id).context("cargo metadata has no root package")?;
    let name = root["name"].as_str().unwrap_or_default().to_owned();

    let lib_name = root["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|target| {
            target["crate_types"]
                .as_array()
                .is_some_and(|types| types.iter().any(|t| t == "cdylib"))
        })
        .and_then(|target| target["name"].as_str())
        .with_context(|| {
            format!(
                "{name} doesn't build a DLL. Add this to its Cargo.toml:\n\n\
                 [lib]\ncrate-type = [\"cdylib\"]\n"
            )
        })?
        .to_owned();

    let copperscript = node(id)
        .and_then(|node| node["deps"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|dep| dep["pkg"].as_str())
        .find(|dep| package(dep).is_some_and(|package| package["name"] == "copperscript"))
        .with_context(|| format!("{name} doesn't depend on copperscript"))?;
    let features: Vec<&str> = node(copperscript)
        .and_then(|node| node["features"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let declared = package(copperscript)
        .and_then(|package| package["metadata"]["copperscript"]["modules"].as_array())
        .context(
            "this copperscript doesn't declare its script modules in \
             [package.metadata.copperscript], so the filter can't tell which modules the pack \
             needs. Use a newer copperscript.",
        )?;
    let mut modules = Vec::new();
    for module in declared {
        let field = |key: &str| {
            module[key].as_str().with_context(|| {
                format!("a module in copperscript's [package.metadata.copperscript] has no {key}")
            })
        };
        if features.contains(&field("feature")?) {
            modules.push(ScriptModule {
                name: field("name")?.to_owned(),
                version: field("version")?.to_owned(),
            });
        }
    }

    Ok(Plugin {
        id: id.to_owned(),
        name,
        lib_name,
        target_dir: PathBuf::from(
            metadata["target_directory"]
                .as_str()
                .context("cargo metadata has no target directory")?,
        ),
        modules,
    })
}

/// Builds the plugin for Windows and returns the path of its DLL.
///
/// Cargo's diagnostics go to stderr, so build errors show up in Regolith's log. Regolith labels
/// every stderr line as an error, so cargo runs with `--quiet` to leave out its progress lines.
pub fn build(
    manifest: &Path,
    plugin: &Plugin,
    profile_args: &[String],
    target_dir: Option<&Path>,
) -> Result<PathBuf> {
    let mut command = cargo(manifest);
    command
        .args(["build", "--quiet", "--manifest-path"])
        .arg(manifest)
        .args([
            "--target",
            TARGET,
            "--message-format=json-render-diagnostics",
        ])
        .args(profile_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(dir) = target_dir {
        command.env("CARGO_TARGET_DIR", dir);
    }
    let mut child = command
        .spawn()
        .context("cannot run cargo, is it on the PATH?")?;

    let mut dll = None;
    let stdout = child.stdout.take().expect("stdout is piped");
    for line in BufReader::new(stdout).lines() {
        let line = line.context("cannot read cargo's output")?;
        if let Ok(message) = serde_json::from_str::<Value>(&line) {
            dll = dll_from_message(&message, &plugin.id).or(dll);
        }
    }
    if !child.wait().context("cannot wait for cargo")?.success() {
        bail!("cargo build failed for {}", plugin.name);
    }
    dll.with_context(|| format!("cargo built {} but reported no DLL", plugin.name))
}

/// Finds the DLL in a `compiler-artifact` message for the plugin's cdylib.
fn dll_from_message(message: &Value, package_id: &str) -> Option<PathBuf> {
    if message["reason"] != "compiler-artifact" || message["package_id"] != package_id {
        return None;
    }
    let kinds = message["target"]["kind"].as_array()?;
    if !kinds.iter().any(|kind| kind == "cdylib") {
        return None;
    }
    message["filenames"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find(|file| {
            Path::new(file)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
        })
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const PLUGIN: &str = "path+file:///repo/examples/hello-world#hello-world@1.0.0";
    const COPPERSCRIPT: &str = "path+file:///repo/crates/copperscript#0.0.0";

    fn metadata(features: &[&str], crate_types: &[&str]) -> Value {
        json!({
            "packages": [
                {
                    "id": PLUGIN,
                    "name": "hello-world",
                    "targets": [{ "name": "hello_world", "kind": ["cdylib"], "crate_types": crate_types }],
                    "metadata": null
                },
                {
                    "id": COPPERSCRIPT,
                    "name": "copperscript",
                    "targets": [],
                    "metadata": { "copperscript": { "modules": [
                        { "feature": "server", "name": "@minecraft/server", "version": "2.11.0" },
                        { "feature": "ui", "name": "@minecraft/server-ui", "version": "2.0.0" }
                    ] } }
                }
            ],
            "resolve": {
                "root": PLUGIN,
                "nodes": [
                    { "id": PLUGIN, "deps": [{ "name": "copperscript", "pkg": COPPERSCRIPT }], "features": [] },
                    { "id": COPPERSCRIPT, "deps": [], "features": features }
                ]
            },
            "target_directory": "/repo/target"
        })
    }

    #[test]
    fn reads_plugin() {
        let plugin = plugin(&metadata(&["common", "default", "server"], &["cdylib"])).unwrap();
        assert_eq!(
            plugin,
            Plugin {
                id: PLUGIN.to_owned(),
                name: "hello-world".to_owned(),
                lib_name: "hello_world".to_owned(),
                target_dir: PathBuf::from("/repo/target"),
                modules: vec![ScriptModule {
                    name: "@minecraft/server".to_owned(),
                    version: "2.11.0".to_owned(),
                }],
            }
        );
    }

    #[test]
    fn modules_follow_features() {
        let plugin = plugin(&metadata(&["server", "ui"], &["cdylib"])).unwrap();
        assert_eq!(plugin.modules.len(), 2);
        let plugin = super::plugin(&metadata(&[], &["cdylib"])).unwrap();
        assert_eq!(plugin.modules, []);
    }

    #[test]
    fn rejects_non_cdylib() {
        let error = plugin(&metadata(&["server"], &["lib"])).unwrap_err();
        assert!(error.to_string().contains("cdylib"));
    }

    #[test]
    fn rejects_missing_copperscript() {
        let mut metadata = metadata(&["server"], &["cdylib"]);
        metadata["resolve"]["nodes"][0]["deps"] = json!([]);
        let error = plugin(&metadata).unwrap_err();
        assert!(error.to_string().contains("doesn't depend on copperscript"));
    }

    #[test]
    fn rejects_old_copperscript() {
        let mut metadata = metadata(&["server"], &["cdylib"]);
        metadata["packages"][1]["metadata"] = Value::Null;
        assert!(plugin(&metadata).is_err());
    }

    #[test]
    fn finds_dll() {
        let message = json!({
            "reason": "compiler-artifact",
            "package_id": PLUGIN,
            "target": { "kind": ["cdylib"] },
            "filenames": [
                "/repo/target/x86_64-pc-windows-msvc/release/hello_world.dll.lib",
                "/repo/target/x86_64-pc-windows-msvc/release/hello_world.dll",
                "/repo/target/x86_64-pc-windows-msvc/release/hello_world.pdb"
            ]
        });
        assert_eq!(
            dll_from_message(&message, PLUGIN),
            Some(PathBuf::from(
                "/repo/target/x86_64-pc-windows-msvc/release/hello_world.dll"
            ))
        );
        assert_eq!(dll_from_message(&message, COPPERSCRIPT), None);
    }

    #[test]
    fn ignores_other_messages() {
        let message = json!({
            "reason": "compiler-artifact",
            "package_id": PLUGIN,
            "target": { "kind": ["lib"] },
            "filenames": ["/repo/target/libhello_world.rlib"]
        });
        assert_eq!(dll_from_message(&message, PLUGIN), None);
        let message = json!({ "reason": "build-finished", "success": true });
        assert_eq!(dll_from_message(&message, PLUGIN), None);
    }
}
