// Vendored from:
//
//   https://crates.io/crates/edit
//   https://crates.io/crates/edit-without-waiting
//
// Originally authored by twilligon and patched by Lucas Garron.
//
// Original source licenced under CC0-1.0

use std::{
    env, fs,
    io::{Error, ErrorKind, Result, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub use tempfile::Builder;
use which::which;

static ENV_VARS: &[&str] = &["VISUAL", "EDITOR"];

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[rustfmt::skip]
static WAITING_HARDCODED_NAMES: &[&str] = &[
    // CLI editors
    "sensible-editor", "nano", "pico", "vim", "nvim", "vi", "emacs",
    // GUI editors
    "code -w", "atom -w", "subl -w", "gedit -w", "gvim",
];

#[cfg(target_os = "macos")]
#[rustfmt::skip]
static WAITING_HARDCODED_NAMES: &[&str] = &[
    // CLI editors
    "nano", "pico", "vim", "nvim", "vi", "emacs",
    // open has a special flag to open in the default text editor
    // (this really should come before the CLI editors, but in order
    // not to break compatibility, we still prefer CLI over GUI)
    "open -Wt",
    "code -w", "atom -w", "subl -w", "gvim", "mate",
];

#[cfg(target_os = "windows")]
#[rustfmt::skip]
static WAITING_HARDCODED_NAMES: &[&str] = &[
    // GUI editors
    "code.cmd -n -w", "atom.exe -w", "subl.exe -w",
    // notepad++ does not block for input
    // Installed by default
    "notepad.exe",
];

fn string_to_cmd(s: String) -> (PathBuf, Vec<String>) {
    match shell_words::split(&s) {
        Ok(mut v) if !v.is_empty() => (v.remove(0).into(), v),
        _ => {
            let mut args = s.split_ascii_whitespace();
            (
                args.next().unwrap().into(),
                args.map(String::from).collect(),
            )
        }
    }
}

fn get_full_editor_cmd(s: String) -> Result<(PathBuf, Vec<String>)> {
    let (path, args) = string_to_cmd(s);
    match which(&path) {
        Ok(result) => Ok((result, args)),
        Err(_) if path.exists() => Ok((path, args)),
        Err(_) => Err(Error::from(ErrorKind::NotFound)),
    }
}

fn get_editor_args() -> Result<(PathBuf, Vec<String>)> {
    ENV_VARS
        .iter()
        .filter_map(env::var_os)
        .filter(|v| !v.is_empty())
        .filter_map(|v| v.into_string().ok())
        .filter_map(|s| get_full_editor_cmd(s).ok())
        .next()
        .or_else(|| {
            WAITING_HARDCODED_NAMES
                .iter()
                .copied()
                .map(str::to_string)
                .filter_map(|s| get_full_editor_cmd(s).ok())
                .next()
        })
        .ok_or_else(|| Error::from(ErrorKind::NotFound))
}

/// Open the contents of a string or buffer in the default editor using a
/// temporary file created with a custom builder.
///
/// This function saves its input to a temporary file created using `builder`,
/// then opens the default editor to it. It waits for the editor to return,
/// re-reads the possibly changed temporary file, and then deletes it.
pub fn edit_with_builder<S: AsRef<[u8]>>(text: S, builder: &Builder) -> Result<String> {
    String::from_utf8(edit_bytes_with_builder(text, builder)?)
        .map_err(|_| Error::from(ErrorKind::InvalidData))
}

fn edit_bytes_with_builder<B: AsRef<[u8]>>(buf: B, builder: &Builder) -> Result<Vec<u8>> {
    let mut file = builder.tempfile()?;
    file.write_all(buf.as_ref())?;

    let path = file.into_temp_path();
    edit_file(&path)?;

    let edited = fs::read(&path)?;

    path.close()?;
    Ok(edited)
}

fn edit_file<P: AsRef<Path>>(file: P) -> Result<()> {
    let (editor, args) = get_editor_args()?;
    let status = Command::new(&editor)
        .args(&args)
        .arg(file.as_ref())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .output()?
        .status;

    if status.success() {
        Ok(())
    } else {
        let full_command = if args.is_empty() {
            format!(
                "{} {}",
                editor.to_string_lossy(),
                file.as_ref().to_string_lossy()
            )
        } else {
            format!(
                "{} {} {}",
                editor.to_string_lossy(),
                args.join(" "),
                file.as_ref().to_string_lossy()
            )
        };

        Err(Error::other(format!(
            "editor '{full_command}' exited with error: {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_editor_commands() {
        let (path, args) = string_to_cmd(
            r#""/Applications/Foo Bar.app/bin/editor" --wait "set ft=gitcommit""#.to_string(),
        );

        assert_eq!(path, PathBuf::from("/Applications/Foo Bar.app/bin/editor"));
        assert_eq!(args, ["--wait", "set ft=gitcommit"]);
    }

    #[test]
    fn waiting_fallbacks_exclude_generic_file_openers() {
        for name in WAITING_HARDCODED_NAMES {
            assert_ne!(*name, "xdg-open");
            assert_ne!(*name, "gnome-open");
            assert_ne!(*name, "kde-open");
            assert_ne!(*name, "open");
            assert_ne!(*name, "open -a TextEdit");
            assert_ne!(*name, "open -a TextMate");
            assert_ne!(*name, "cmd.exe /C start");
        }
    }
}
