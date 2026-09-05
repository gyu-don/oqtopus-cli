//! Native backend commands and their result data.

use std::ffi::OsString;
use std::fs;
use std::path::Path;

use crate::metadata::{metadata_get, migrate_metadata_keys};

/// Validated metadata. Retain source bytes because text output must preserve unknown fields,
/// line endings, and non-UTF-8 bytes. A future JSON view must define its own parsing contract.
pub(crate) struct BackendInfo {
    pub(crate) metadata: Vec<u8>,
}

/// Validates the current backend environment and returns its metadata.
pub(crate) fn backend_info(args: &[OsString]) -> Result<BackendInfo, String> {
    if !args.is_empty() {
        return Err("oqtopus backend info does not accept arguments.".to_owned());
    }

    let path = Path::new(".metadata");
    if !path.is_file() {
        return Err(
            ".metadata not found.\nThis directory is not an OQTOPUS backend environment."
                .to_owned(),
        );
    }

    migrate_metadata_keys(path);
    // Parse through a lossy view, but retain the original bytes for output. This preserves the
    // legacy command's byte-for-byte behavior after the fields required for validation are found.
    // The legacy metadata lookup also reports an unreadable file as a missing required key.
    let contents = fs::read(path).map_err(|_| "invalid .metadata: missing template.".to_owned())?;
    let text = String::from_utf8_lossy(&contents);

    let template = metadata_get(&text, "template")
        .ok_or_else(|| "invalid .metadata: missing template.".to_owned())?;
    if template != "backend" {
        return Err(format!(
            "invalid environment template. Found template='{template}', but 'oqtopus backend' requires template='backend'."
        ));
    }

    let environment_root = metadata_get(&text, "environment_root")
        .or_else(|| metadata_get(&text, "env_root"))
        .ok_or_else(|| "invalid .metadata: missing environment_root.".to_owned())?;
    let current = fs::canonicalize(".")
        .map_err(|error| format!("failed to resolve current directory: {error}"))?;
    if environment_root != current.to_string_lossy() {
        return Err(format!(
            "Current directory does not match environment_root.\nenvironment_root = {environment_root}\ncurrent          = {}",
            current.display()
        ));
    }

    metadata_get(&text, "install_root")
        .ok_or_else(|| "invalid .metadata: missing install_root.".to_owned())?;

    Ok(BackendInfo { metadata: contents })
}
