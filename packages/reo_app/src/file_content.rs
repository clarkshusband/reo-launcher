#![allow(dead_code)]

use std::path::Path;

pub fn single_line(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn desktop_exec_arg(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            // The value-level unescape runs before the Exec quoting one, so the
            // backslash the shell layer wants has to survive doubled
            '"' | '$' | '`' => {
                out.push_str(r"\\");
                out.push(c);
            }
            '\\' => out.push_str(r"\\\\"),
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn desktop_entry(name: &str, exe: &Path, folder: &str, icon: &str) -> String {
    let name = single_line(name);
    let exec = format!(
        "{} --launch {}",
        desktop_exec_arg(&exe.to_string_lossy()),
        desktop_exec_arg(folder),
    );

    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name={name}\n\
         Comment=Launch {name} with Reo Launcher\n\
         Exec={exec}\n\
         Icon={icon}\n\
         Terminal=false\n\
         Categories=Game;\n\
         StartupNotify=true\n"
    )
}

pub fn url_handler_entry(exe: &Path, scheme: &str, icon: &str) -> String {
    let exec = desktop_exec_arg(&exe.to_string_lossy());

    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Reo Launcher\n\
         Exec={exec} %u\n\
         Icon={icon}\n\
         Terminal=false\n\
         NoDisplay=true\n\
         MimeType=x-scheme-handler/{scheme};\n"
    )
}

pub fn url_shortcut(url: &str, icon: &Path) -> String {
    format!(
        "[InternetShortcut]\r\nURL={url}\r\nIconFile={}\r\nIconIndex=0\r\n",
        icon.display(),
    )
}

/// Windows reads an INI file as the ANSI code page unless it opens with a
/// UTF-16LE byte order mark, which a non-ASCII install path needs
pub fn utf16le_with_bom(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_newline_in_a_cluster_name_cannot_forge_a_second_key() {
        let entry = desktop_entry(
            "Pack\nExec=/bin/sh",
            &PathBuf::from("/usr/bin/reo_app"),
            "pack",
            "reo_app",
        );
        assert!(!entry.lines().any(|line| line.starts_with("Exec=/bin/sh")));
        assert!(entry.contains(r#"Exec="/usr/bin/reo_app" --launch "pack""#));
        assert!(entry.contains("Name=Pack Exec=/bin/sh"));
    }

    #[test]
    fn a_space_in_the_install_path_stays_one_argument() {
        let entry = desktop_entry(
            "Pack",
            &PathBuf::from("/opt/One Client/reo_app"),
            "my pack",
            "reo_app",
        );
        assert!(entry.contains(r#"Exec="/opt/One Client/reo_app" --launch "my pack""#));
    }

    #[test]
    fn the_url_handler_quotes_an_install_path_with_a_space() {
        let entry = url_handler_entry(
            &PathBuf::from("/opt/One Client/reo_app"),
            "reolauncher",
            "reo_app",
        );
        assert!(entry.contains(r#"Exec="/opt/One Client/reo_app" %u"#));
        assert!(entry.contains("MimeType=x-scheme-handler/reolauncher;"));
    }

    #[test]
    fn the_url_handler_keeps_the_field_code_outside_the_quoted_path() {
        let entry = url_handler_entry(&PathBuf::from("/usr/bin/reo_app"), "reolauncher", "reo_app");
        assert!(entry.contains(r#"Exec="/usr/bin/reo_app" %u"#));
        assert!(!entry.contains("%%u"));
    }

    #[test]
    fn percent_is_doubled_for_the_desktop_spec() {
        assert_eq!(desktop_exec_arg("100%"), r#""100%%""#);
    }

    #[test]
    fn desktop_reserved_characters_are_escaped() {
        assert_eq!(desktop_exec_arg("a$b`c\"d"), r#""a\\$b\\`c\\"d""#);
    }

    #[test]
    fn an_install_path_with_a_backslash_survives_both_unescape_passes() {
        let entry = desktop_entry(
            "Pack",
            &PathBuf::from(r"/opt/one\client/$app"),
            "pack",
            "reo_app",
        );
        assert!(entry.contains(r#"Exec="/opt/one\\\\client/\\$app" --launch "pack""#));
    }

    #[test]
    fn the_url_shortcut_is_a_single_ini_section() {
        let file = url_shortcut(
            "reolauncher://launch/My%20Pack",
            &PathBuf::from(r"C:\Program Files\Reo Launcher\reo_app.exe"),
        );

        assert!(file.starts_with("[InternetShortcut]\r\n"));
        assert!(file.contains("\r\nURL=reolauncher://launch/My%20Pack\r\n"));
        assert!(file.contains(r"IconFile=C:\Program Files\Reo Launcher\reo_app.exe"));
        assert!(file.ends_with("IconIndex=0\r\n"));
    }

    #[test]
    fn a_non_ascii_path_survives_the_utf16_encoding() {
        let bytes = utf16le_with_bom("IconFile=C:\\Użytkownik\r\n");

        assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(
            String::from_utf16(&units).unwrap(),
            "IconFile=C:\\Użytkownik\r\n"
        );
    }
}
