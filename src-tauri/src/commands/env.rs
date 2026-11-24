//! Reading a `.env` file off disk for the env kind. A dropped or picked path
//! comes in; its text goes back out to become the draft's body. The file is
//! never parsed here — the frontend owns the format — and never logged: the
//! contents are the secret.

use std::path::Path;

use serde::Serialize;
use tauri::State;

use crate::error::{Error, Result};
use crate::grants::{PathGrants, Purpose};
use crate::state::AppState;
use crate::storage::read_regular_file_capped;

// A real .env is a few kilobytes. Anything past this is not one, and reading
// it whole into the webview would only ever be a mistake.
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnvFile {
    pub file_name: String,
    pub body: String,
}

/// `.env`, `.env.local`, `production.env` — what the env kind is named like.
/// Mirrors `isEnvFileName` in `src/kinds/env/ingest.ts`, which is what the
/// drop target and the editor decide by.
fn is_env_file_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == ".env"
        || name
            .strip_prefix(".env.")
            .is_some_and(|rest| !rest.is_empty())
        || name.ends_with(".env")
}

/// One `KEY=value` line — `[A-Za-z_][A-Za-z0-9_]*` up to the `=`, after an
/// optional `export`. The value is not read: whether the line *is* a variable
/// is all this decides.
fn is_var_line(line: &str) -> bool {
    let rest = match line.split_once(char::is_whitespace) {
        Some(("export", tail)) => tail.trim_start(),
        _ => line,
    };
    let Some((key, _)) = rest.split_once('=') else {
        return false;
    };
    let key = key.trim_end();
    !key.is_empty()
        && key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether text reads as a block of variables rather than as some other file —
/// the `looksLikeEnv` the frontend claims a drop by (`src/kinds/env/parse.ts`),
/// ported: more than one line, and at least one variable among them.
///
/// Stricter in one way, deliberately: every line that is not blank and not a
/// comment has to be a variable. The frontend asks only that *some* line is,
/// which a PEM key satisfies the moment its base64 ends in `=` padding, and
/// this is the check standing between the webview and a file it did not name.
/// The cost is a file whose values are quoted across several lines — and that
/// file is accepted by its name, like every other `.env`.
fn looks_like_env(text: &str) -> bool {
    let mut vars = 0;
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !is_var_line(line) {
            return false;
        }
        vars += 1;
    }
    vars > 0 && text.trim().contains('\n')
}

// The command's body, split out so the refusals can be tested without a Tauri
// runtime around them.
pub fn read_env_text(path: &Path) -> Result<EnvFile> {
    let bytes = read_regular_file_capped(path, MAX_BYTES)?;
    let body = String::from_utf8(bytes).map_err(|_| Error::FileNotText)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Whatever path arrives is read whole and handed back, so what leaves here
    // is held to being an env file: named like one, or — since the drop target
    // claims a file that merely reads like one — written like one. A key, a
    // password database, a shell history is neither, and stays on disk.
    if !is_env_file_name(&file_name) && !looks_like_env(&body) {
        return Err(Error::Unsupported(format!(
            "{file_name} is not a .env file"
        )));
    }
    Ok(EnvFile { file_name, body })
}

// Small as `.env` files are, this is still a `stat` and a read of a path the
// user just pointed at — which may be a network mount. Off the IPC thread with
// everything else that touches the disk.
#[tauri::command]
pub async fn read_env_file(
    path: String,
    state: State<'_, AppState>,
    grants: State<'_, PathGrants>,
) -> Result<EnvFile> {
    // Whatever path comes in is read whole and handed to the webview, so this is
    // a read of the user's disk on the webview's say-so. Only an open vault may
    // ask for one — the drop target that calls this lives in the unlocked shell,
    // and a locked app has no business reading files for anybody — and only for
    // a path the user chose as an env file: dropped on the window or picked
    // through the env `pick_file`, either of which granted it for this read
    // (see `grants`). The name is gated too, in `read_env_text`. A lock while
    // the file is read discards the text: the session that asked for it is
    // gone.
    let epoch = super::unlocked_epoch(&state)?;
    if !grants.take(Path::new(&path), Purpose::Env) {
        return Err(crate::grants::not_chosen());
    }
    let file = super::blocking(move || read_env_text(Path::new(&path))).await?;
    super::same_session(&state, epoch)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // The file keeps the name the test asked for — the name is what is being
    // tested — so the per-process part is the directory.
    fn scratch(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rowel-env-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn reads_a_small_utf8_file_with_its_name() {
        let path = scratch(".env.production", b"A=1\nB=two\n");
        let file = read_env_text(&path).unwrap();
        assert_eq!(file.body, "A=1\nB=two\n");
        assert_eq!(file.file_name, ".env.production");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn takes_every_shape_of_env_name() {
        for name in [
            ".env",
            ".env.local",
            ".env.production",
            "production.env",
            "LOCAL.ENV",
        ] {
            assert!(is_env_file_name(name), "{name}");
        }
    }

    #[test]
    fn refuses_a_file_that_is_not_named_like_an_env() {
        for name in [".envrc", "notes.txt", "id_ed25519", ".env.", "env"] {
            assert!(!is_env_file_name(name), "{name}");
        }
    }

    fn refusal(name: &str, bytes: &[u8]) {
        let path = scratch(name, bytes);
        let err = read_env_text(&path).unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)), "{name}: {err}");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn refuses_a_private_key() {
        refusal(
            "id_ed25519",
            b"-----BEGIN OPENSSH PRIVATE KEY-----\n\
              b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gt\n\
              ZWQyNTUxOQAAACDkQ8NRAAAAtHNzaC1lZDI1NTE5AAAAIORDw1E=\n\
              -----END OPENSSH PRIVATE KEY-----\n",
        );
    }

    #[test]
    fn refuses_prose_and_other_formats() {
        refusal("notes.txt", b"hello\nworld\n");
        refusal("config.json", b"{\n  \"token\": \"abc\"\n}\n");
        refusal(
            "passwd",
            b"root:*:0:0:System Administrator:/var/root:/bin/sh\n",
        );
    }

    // The drop target claims a file that reads like an env whatever it is
    // called, so the backend has to hand that same file back.
    #[test]
    fn takes_an_oddly_named_file_written_like_an_env() {
        let path = scratch("keys", b"# staging\nexport A=1\nDB_URL=postgres://x\n");
        let file = read_env_text(&path).unwrap();
        assert!(file.body.contains("DB_URL"));
        let _ = fs::remove_file(path);
    }

    // A name on the allowlist is enough by itself — an env file may hold
    // anything, including nothing.
    #[test]
    fn takes_an_env_name_whatever_is_in_it() {
        let path = scratch(".env.local", b"not a variable in sight\n");
        assert!(read_env_text(&path).is_ok());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn refuses_a_file_over_the_cap() {
        let path = scratch("big.env", &vec![b'x'; (MAX_BYTES + 1) as usize]);
        let err = read_env_text(&path).unwrap_err();
        assert!(matches!(err, Error::FileTooLarge), "{err}");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn refuses_bytes_that_are_not_utf8() {
        let path = scratch("binary.env", &[0xff, 0xfe, b'A', b'=', b'1']);
        let err = read_env_text(&path).unwrap_err();
        assert!(matches!(err, Error::FileNotText), "{err}");
        let _ = fs::remove_file(path);
    }
}
