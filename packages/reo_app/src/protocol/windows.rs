use std::path::Path;

use anyhow::Result;

use super::SCHEME;
use super::registry::{read_string, write_string};

fn root_key() -> String {
    format!(r"Software\Classes\{SCHEME}")
}

fn command_key() -> String {
    format!(r"{}\shell\open\command", root_key())
}

pub fn register(exe: &Path) -> Result<()> {
    let exe = exe.display().to_string();
    let root = root_key();

    // Each value is checked on its own since any one of them going missing
    // breaks the handler, the command goes last as `is_registered` reads it
    let values = [
        (root.clone(), None, format!("URL:{SCHEME} Protocol")),
        (root.clone(), Some("URL Protocol"), String::new()),
        (format!(r"{root}\DefaultIcon"), None, format!("{exe},0")),
        (command_key(), None, format!("\"{exe}\" \"%1\"")),
    ];

    let mut repaired = false;
    for (subkey, name, value) in &values {
        if read_string(subkey, *name).as_deref() != Some(value.as_str()) {
            write_string(subkey, *name, value)?;
            repaired = true;
        }
    }

    if repaired {
        tracing::info!(scheme = SCHEME, "registered the url scheme");
    }
    Ok(())
}

pub fn is_registered() -> bool {
    read_string(&command_key(), None).is_some_and(|value| !value.is_empty())
}
