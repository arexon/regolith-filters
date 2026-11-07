//! A Regolith filter that builds a Copperscript plugin crate into the behavior pack.
//!
//! Regolith runs it with the working directory at `.regolith/tmp`, which holds copies of the
//! packs, and sets `ROOT_DIR` to the project root.

mod cargo;
mod manifest;
mod project;
mod settings;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::settings::Settings;

const MANIFEST: &str = "BP/manifest.json";

fn run() -> Result<()> {
    let settings = Settings::parse(env::args().nth(1).as_deref())?;
    let root = PathBuf::from(
        env::var_os("ROOT_DIR").context("ROOT_DIR is not set. Run this filter through Regolith")?,
    );

    // Build the original crate. Relative paths in the copy under `.regolith/tmp` would break, and
    // the copy would rebuild from scratch every time.
    let data_dir = project::data_dir(&root)?;
    let crate_dir = data_dir.join(&settings.crate_dir);
    let cargo_toml = crate_dir.join("Cargo.toml");
    if !cargo_toml.is_file() {
        bail!(
            "there's no Cargo.toml in {}. Put the plugin crate there, or set the filter's \
             \"crate\" setting to its folder inside the data folder",
            crate_dir.display()
        );
    }

    let plugin = cargo::metadata(&cargo_toml)?;
    let dll_name = format!("{}.dll", plugin.lib_name);

    // Update the manifest before building, so a mistake in it shows up before a long build. It's
    // written only once the build succeeds.
    let text = fs::read_to_string(MANIFEST).with_context(|| {
        format!("cannot read {MANIFEST}. The behavior pack needs a hand-written manifest")
    })?;
    let mut manifest: Value = serde_json::from_str(&text).with_context(|| {
        format!("{MANIFEST} isn't valid JSON. Comments and trailing commas aren't supported")
    })?;
    manifest::patch(
        &mut manifest,
        &format!("scripts/{dll_name}"),
        &plugin.name,
        &plugin.modules,
    )
    .with_context(|| format!("cannot update {MANIFEST}"))?;

    let target_dir = project::target_dir_override(&root, &data_dir, &plugin.target_dir);
    println!(
        "building {} with the {} profile",
        plugin.name, settings.profile
    );
    let dll = cargo::build(
        &cargo_toml,
        &plugin,
        &settings.profile_args(),
        target_dir.as_deref(),
    )?;

    let scripts = Path::new("BP/scripts");
    fs::create_dir_all(scripts).context("cannot create BP/scripts")?;
    fs::copy(&dll, scripts.join(&dll_name))
        .with_context(|| format!("cannot copy {} into BP/scripts", dll.display()))?;
    fs::write(MANIFEST, serde_json::to_string_pretty(&manifest)? + "\n")
        .with_context(|| format!("cannot write {MANIFEST}"))?;

    let mut summary = vec![format!("built {dll_name}")];
    for module in &plugin.modules {
        summary.push(format!("{} {}", module.name, module.version));
    }
    println!("{}", summary.join(", "));
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
