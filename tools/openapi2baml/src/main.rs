//! `openapi2baml --config <file> [--root <dir>] [--check]`
//!
//! Writes the generated BAML (and the coverage report) the config describes.
//! `--check` writes nothing and fails if any owned file is missing, stale or
//! extra.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::ExitCode,
};

use openapi2baml::{config::Config, run};

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("openapi2baml: {message}");
            ExitCode::FAILURE
        }
    }
}

fn real_main() -> Result<(), String> {
    let mut config_path = None;
    let mut root = PathBuf::from(".");
    let mut check = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config_path = args.next(),
            "--root" => root = PathBuf::from(args.next().ok_or("--root needs a value")?),
            "--check" => check = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let config_path = config_path.ok_or("--config is required")?;
    let text = std::fs::read_to_string(root.join(&config_path)).map_err(|e| format!("read {config_path}: {e}"))?;
    let config: Config = serde_json::from_str(&text).map_err(|e| format!("parse {config_path}: {e}"))?;
    let output = run(&root, &config).map_err(|e| e.to_string())?;

    // Every .baml file currently in an owned directory.
    let mut existing = BTreeSet::new();
    for dir in &output.owned_dirs {
        collect_baml(&root, Path::new(dir), &mut existing).map_err(|e| format!("scan {dir}: {e}"))?;
    }
    let wanted: BTreeSet<String> = output.files.keys().cloned().collect();

    if check {
        let mut problems = Vec::new();
        for (path, text) in &output.files {
            match std::fs::read_to_string(root.join(path)) {
                Ok(current) if current == *text => {}
                Ok(_) => problems.push(format!("stale: {path}")),
                Err(_) => problems.push(format!("missing: {path}")),
            }
        }
        problems.extend(existing.difference(&wanted).map(|p| format!("extra: {p}")));
        if !problems.is_empty() {
            return Err(format!("generated files are out of date (run scripts/import.sh):\n  {}", problems.join("\n  ")));
        }
        println!("openapi2baml: {} files up to date", output.files.len());
        return Ok(());
    }

    for stale in existing.difference(&wanted) {
        std::fs::remove_file(root.join(stale)).map_err(|e| format!("remove {stale}: {e}"))?;
    }
    for (path, text) in &output.files {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        std::fs::write(&full, text).map_err(|e| format!("write {path}: {e}"))?;
    }
    println!("openapi2baml: wrote {} files", output.files.len());
    Ok(())
}

fn collect_baml(root: &Path, dir: &Path, out: &mut BTreeSet<String>) -> std::io::Result<()> {
    let full = root.join(dir);
    if !full.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(full)? {
        let entry = entry?;
        let rel = dir.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            collect_baml(root, &rel, out)?;
        } else if rel.extension().is_some_and(|e| e == "baml") {
            // The importer's own paths use `/` on every platform.
            out.insert(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
