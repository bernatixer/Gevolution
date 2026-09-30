//! Loading rule packages and scenarios from disk with bounded input sizes.
//!
//! Imported documents are untrusted: sizes are checked before parsing and
//! everything is validated by the compiler before it can affect a world.

use crate::schema::{Package, Scenario};
use std::path::{Path, PathBuf};

pub const MAX_PACKAGE_BYTES: u64 = 8 << 20;
pub const MAX_SCENARIO_BYTES: u64 = 64 << 20;

pub fn read_bounded(path: &Path, max: u64) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > max {
        return Err(format!("{} is {} bytes, over the {max} byte limit", path.display(), meta.len()));
    }
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn parse_package(text: &str) -> Result<Package, String> {
    if text.len() as u64 > MAX_PACKAGE_BYTES {
        return Err("package exceeds size limit".into());
    }
    serde_json::from_str(text).map_err(|e| format!("invalid package: {e}"))
}

pub fn parse_scenario(text: &str) -> Result<Scenario, String> {
    if text.len() as u64 > MAX_SCENARIO_BYTES {
        return Err("scenario exceeds size limit".into());
    }
    serde_json::from_str(text).map_err(|e| format!("invalid scenario: {e}"))
}

pub fn load_package(path: &Path) -> Result<Package, String> {
    parse_package(&read_bounded(path, MAX_PACKAGE_BYTES)?).map_err(|e| format!("{}: {e}", path.display()))
}

/// Repository asset root: `$EVOLVING_WORLDS_ASSETS`, else `assets/` found by walking up from the cwd.
pub fn asset_root() -> PathBuf {
    if let Ok(p) = std::env::var("EVOLVING_WORLDS_ASSETS") {
        return PathBuf::from(p);
    }
    let mut dir = std::env::current_dir().unwrap_or_default();
    loop {
        let cand = dir.join("assets");
        if cand.join("rules").is_dir() {
            return cand;
        }
        if !dir.pop() {
            return PathBuf::from("assets");
        }
    }
}

/// Load a scenario and the packages it names (resolved in `<assets>/rules/**`).
pub fn load_scenario(path: &Path) -> Result<(Scenario, Vec<Package>), String> {
    let sc = parse_scenario(&read_bounded(path, MAX_SCENARIO_BYTES)?).map_err(|e| format!("{}: {e}", path.display()))?;
    let root = asset_root().join("rules");
    let mut pkgs = vec![];
    for name in &sc.packages {
        if name.contains("..") || name.starts_with('/') {
            return Err(format!("package path {name} must stay inside the rules directory"));
        }
        pkgs.push(load_package(&root.join(name))?);
    }
    Ok((sc, pkgs))
}
