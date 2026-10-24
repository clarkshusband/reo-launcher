use std::path::Path;

use anyhow::{Result, bail};

use super::ShortcutRequest;
use crate::file_content::{url_shortcut, utf16le_with_bom};
use crate::protocol;

pub const EXTENSION: &str = "url";

pub fn write(request: &ShortcutRequest, exe: &Path, path: &Path) -> Result<()> {
    if !protocol::is_registered() {
        bail!(
            "the {} :// handler is not registered on this machine",
            protocol::SCHEME
        );
    }

    let url = protocol::launch_url(&request.folder_name);
    let content = url_shortcut(&url, exe);
    // Plain ASCII reads the same in every code page so only a path that needs it
    // pays for the UTF-16 file
    if content.is_ascii() {
        std::fs::write(path, content)?;
    } else {
        std::fs::write(path, utf16le_with_bom(&content))?;
    }

    Ok(())
}
