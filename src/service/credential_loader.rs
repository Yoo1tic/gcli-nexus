use std::path::Path;
use std::{fs, io};

use serde_json::Value;
use tracing::{info, warn};

use crate::google_oauth::credentials::GoogleCredential;

/// Load credential JSON files from a directory into GoogleCredential structs.
pub fn load_from_dir(dir: &Path) -> io::Result<Vec<GoogleCredential>> {
    if !dir.exists() {
        info!(path = %dir.display(), "credentials directory not found; skipping load");
        return Ok(Vec::new());
    }
    let iter = fs::read_dir(dir)?;
    let loaded: Vec<GoogleCredential> = iter
        .filter_map(|entry| match entry {
            Ok(e) => Some(e.path()),
            Err(e) => {
                warn!(error = %e, "failed to read credentials dir entry");
                None
            }
        })
        .filter(|path| is_json_file(path.as_path()))
        .filter_map(|path| read_file(&path).map(|contents| (path, contents)))
        .filter_map(|(path, contents)| parse_json(&path, &contents).map(|value| (path, value)))
        .filter_map(
            |(path, value)| match GoogleCredential::from_payload(&value) {
                Ok(c) => Some(c),
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "failed to normalize credential");
                    None
                }
            },
        )
        .collect();
    Ok(loaded)
}

fn is_json_file(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("json"))
        == Some(true)
}

fn read_file(path: &Path) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Some(contents),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to read credential file");
            None
        }
    }
}

fn parse_json(path: &Path, contents: &str) -> Option<Value> {
    match serde_json::from_str::<Value>(contents) {
        Ok(value) => Some(value),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "invalid credential JSON");
            None
        }
    }
}
