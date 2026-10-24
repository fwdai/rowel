//! The agent key: this machine's identification as an agent, which is the
//! whole credential in Tier 1 (`spec/02`). Only the public half is ever used
//! — `associate` and `test-associate` carry it, the app stores it against a
//! name — so only the public half is kept, in a file of the user's own in
//! the config directory. The name beside it is the one the app's dialog gave
//! the agent, which `test-associate` has to repeat.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rowel_broker::client::Channel;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentKey {
    /// The X25519 public key, base64, as the protocol carries it.
    pub key: String,
    /// The name the app knows this key under, once it does.
    pub name: String,
}

/// `<config dir>/rowel/agent.json`: `~/Library/Application Support/rowel/agent.json`
/// on macOS, `$XDG_CONFIG_HOME/rowel/agent.json` on Linux,
/// `%APPDATA%\rowel\agent.json` on Windows.
pub fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("rowel").join("agent.json"))
}

/// The key file at `path`, or `None` when there is none yet. A file that is
/// there but does not read is an error, not a missing file: minting over it
/// would silently orphan an association the user already approved.
pub fn load(path: &Path) -> io::Result<Option<AgentKey>> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    serde_json::from_slice(&raw).map(Some).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not an agent key file: {e}", path.display()),
        )
    })
}

/// The key file at `path`, minted under `name` when there is none yet.
pub fn load_or_mint(path: &Path, name: &str) -> io::Result<AgentKey> {
    if let Some(key) = load(path)? {
        return Ok(key);
    }
    let key = AgentKey {
        key: Channel::new().public_key(),
        name: name.to_string(),
    };
    save(path, &key)?;
    Ok(key)
}

/// Write the key file, readable by its owner alone: the key is only a public
/// one, but it is the credential, and whoever holds it is this agent.
pub fn save(path: &Path, key: &AgentKey) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        private_dir(dir)?;
    }
    let raw = serde_json::to_vec_pretty(key)?;
    private_file(path)?.write_all(&raw)
}

#[cfg(unix)]
fn private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(dir)
}

#[cfg(unix)]
fn private_file(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

// Windows has no mode bits; a file under `%APPDATA%` is the user's by the
// profile's own ACL, which is what the owner-only mode means on Unix.
#[cfg(not(unix))]
fn private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> io::Result<fs::File> {
    fs::File::create(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_minted_once_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rowel").join("agent.json");
        assert_eq!(load(&path).unwrap(), None);

        let minted = load_or_mint(&path, "Claude Code").unwrap();
        assert_eq!(minted.name, "Claude Code");
        assert!(!minted.key.is_empty());
        assert_eq!(load(&path).unwrap(), Some(minted.clone()));
        assert_eq!(load_or_mint(&path, "Other").unwrap(), minted);

        let renamed = AgentKey {
            name: "Claude".into(),
            ..minted
        };
        save(&path, &renamed).unwrap();
        assert_eq!(load(&path).unwrap(), Some(renamed));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_its_directory_are_the_owners_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rowel").join("agent.json");
        load_or_mint(&path, "Agent").unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
    }

    #[test]
    fn a_file_that_is_not_a_key_is_an_error_not_a_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.json");
        fs::write(&path, b"{}").unwrap();
        let error = load(&path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(load_or_mint(&path, "Agent").is_err());
    }
}
