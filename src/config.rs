//! The profile store: every host this tool can talk to, and which one is default.
//!
//! Deliberately a plain JSON file rather than a config crate — it holds
//! credentials, so the only requirements are "readable by a human when something
//! goes wrong" and "not world-readable".

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub url: String,
    pub token: String,
    #[serde(default)]
    pub default: bool,
}

pub struct ProfileStore {
    path: PathBuf,
}

impl ProfileStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `$XDG_CONFIG_HOME/{binary name}/profiles.json`, or `~/.config/…` when that
    /// is unset.
    pub fn default_path() -> PathBuf {
        Self::config_path(
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")),
            crate::APP_NAME,
        )
    }

    /// Split out from `default_path` so the precedence can be tested without
    /// setting process-wide environment variables from a parallel test run.
    fn config_path(xdg: Option<OsString>, home: Option<OsString>, app: &str) -> PathBuf {
        let base = match xdg {
            // The spec says a relative (or empty) XDG_CONFIG_HOME is invalid and
            // must be ignored. Honouring one would put a credentials file at a
            // path that depends on the shell's working directory — a different
            // file per directory the tool is run from.
            Some(x) if Path::new(&x).is_absolute() => PathBuf::from(x),
            _ => PathBuf::from(home.unwrap_or_else(|| ".".into())).join(".config"),
        };
        base.join(app).join("profiles.json")
    }

    /// Every profile, for READ paths; empty if the file is corrupt.
    ///
    /// Safe here: worst case the user sees an empty list. NOT safe for writes —
    /// use `try_all()` there.
    pub fn all(&self) -> Vec<Profile> {
        self.try_all().unwrap_or_default()
    }

    /// Every profile, erroring if the file EXISTS but cannot be read.
    ///
    /// Must be used by every path that saves: add/remove/set_default read then
    /// write the whole list back, so treating a corrupt file as "empty" would make
    /// the next command write a fresh list and DELETE every profile — along with
    /// tokens that cannot be recovered from anywhere else. A missing file really
    /// does mean empty; a corrupt one must stop the write.
    pub fn try_all(&self) -> Result<Vec<Profile>> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(anyhow!(
                    "cannot read {}: {e}. Fix or move that file; continuing would \
                     overwrite it and delete every profile.",
                    self.path.display()
                ))
            }
        };
        if raw.trim().is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_str(&raw).map_err(|e| {
            anyhow!(
                "{} is not valid JSON: {e}. Fix or move that file; continuing would \
                 overwrite it and delete every profile.",
                self.path.display()
            )
        })
    }

    pub fn get(&self, name: &str) -> Option<Profile> {
        self.all().into_iter().find(|p| p.name == name)
    }

    /// The default profile, or the only one when nothing is marked.
    pub fn default(&self) -> Option<Profile> {
        let all = self.all();
        all.iter()
            .find(|p| p.default)
            .or_else(|| all.first())
            .cloned()
    }

    pub fn add(&self, profile: Profile) -> Result<()> {
        let mut all = self.try_all()?;
        if all.iter().any(|p| p.name == profile.name) {
            return Err(anyhow!("Profile '{}' already exists", profile.name));
        }
        // The first profile added is the default: a tool with exactly one host and
        // no default would refuse to do anything until told the obvious.
        let first = all.is_empty();
        all.push(Profile {
            default: profile.default || first,
            ..profile
        });
        self.save(&mut all)
    }

    pub fn update(&self, name: &str, url: Option<String>, token: Option<String>) -> Result<()> {
        let mut all = self.try_all()?;
        let p = all
            .iter_mut()
            .find(|p| p.name == name)
            .ok_or_else(|| anyhow!("Profile '{name}' not found"))?;
        if let Some(url) = url {
            p.url = url;
        }
        if let Some(token) = token {
            p.token = token;
        }
        self.save(&mut all)
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        let mut all = self.try_all()?;
        let before = all.len();
        all.retain(|p| p.name != name);
        if all.len() == before {
            return Err(anyhow!("Profile '{name}' not found"));
        }
        self.save(&mut all)
    }

    pub fn set_default(&self, name: &str) -> Result<()> {
        let mut all = self.try_all()?;
        if !all.iter().any(|p| p.name == name) {
            return Err(anyhow!("Profile '{name}' not found"));
        }
        for p in &mut all {
            p.default = p.name == name;
        }
        self.save(&mut all)
    }

    /// Write the list back, with exactly one default.
    ///
    /// Enforced here rather than at each call site: two defaults and "which host
    /// am I talking to?" has no answer, and the wrong one is a destructive command
    /// on the wrong machine.
    fn save(&self, all: &mut [Profile]) -> Result<()> {
        if !all.is_empty() && !all.iter().any(|p| p.default) {
            all[0].default = true;
        }
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(&all)?;

        // Written to a sibling temp file and renamed into place, for two reasons.
        //
        // The mode is set AT CREATION. Writing the file and then chmod'ing it
        // leaves the tokens readable by every other user on the machine for
        // however long those two syscalls are apart — brief, but a credentials
        // file has no acceptable brief.
        //
        // And `rename` within a directory is atomic, so an interrupted save
        // leaves the PREVIOUS file intact. Writing in place truncates first: a
        // crash between truncate and write produces exactly the empty-or-partial
        // file that `try_all` refuses to overwrite, locking the user out of their
        // own profiles until they edit it by hand.
        let tmp = self.path.with_extension("json.tmp");
        // A temp file left by an earlier crash would be reused with ITS mode,
        // not ours, so it is removed rather than truncated.
        let _ = fs::remove_file(&tmp);
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        // Windows has no mode() here. The per-user profile directory this path
        // lives under is already restricted to that user by its inherited ACL.
        let mut file = opts.open(&tmp)?;
        file.write_all(json.as_bytes())?;
        // Flushed before the rename, or the rename can be durable while the
        // contents behind it are not — a valid name pointing at a truncated file.
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ProfileStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("profiles.json"));
        (dir, store)
    }

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.into(),
            url: format!("https://{name}.example"),
            token: "t".into(),
            default: false,
        }
    }

    #[test]
    fn a_missing_file_is_empty_not_an_error() {
        let (_dir, store) = store();
        assert!(store.try_all().unwrap().is_empty());
        assert!(store.default().is_none());
    }

    #[test]
    fn the_first_profile_becomes_the_default() {
        let (_dir, store) = store();
        store.add(profile("a")).unwrap();
        store.add(profile("b")).unwrap();
        assert_eq!(store.default().unwrap().name, "a");
    }

    #[test]
    fn there_is_never_more_than_one_default() {
        let (_dir, store) = store();
        store.add(profile("a")).unwrap();
        store.add(profile("b")).unwrap();
        store.set_default("b").unwrap();
        assert_eq!(store.all().iter().filter(|p| p.default).count(), 1);
        assert_eq!(store.default().unwrap().name, "b");
    }

    #[test]
    fn removing_the_default_promotes_another_one() {
        // Without this the tool would hold two profiles and refuse to pick either.
        let (_dir, store) = store();
        store.add(profile("a")).unwrap();
        store.add(profile("b")).unwrap();
        store.remove("a").unwrap();
        assert_eq!(store.default().unwrap().name, "b");
    }

    /// The file holds API tokens. `0644` for even an instant means any other
    /// account on the machine could have read them.
    #[cfg(unix)]
    #[test]
    fn the_token_file_is_never_readable_by_anyone_else() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, store) = store();
        store.add(profile("a")).unwrap();
        let mode = fs::metadata(dir.path().join("profiles.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "got {:o}", mode & 0o777);
    }

    /// A temp file surviving a previous crash must not donate its permissions to
    /// the next save — reusing it is how a 0644 file comes back after the bug
    /// that created it was fixed.
    #[cfg(unix)]
    #[test]
    fn a_stale_temp_file_does_not_leak_its_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, store) = store();
        let tmp = dir.path().join("profiles.json.tmp");
        fs::write(&tmp, "leftover").unwrap();
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)).unwrap();

        store.add(profile("a")).unwrap();

        let mode = fs::metadata(dir.path().join("profiles.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "got {:o}", mode & 0o777);
        assert!(!tmp.exists(), "the temp file was left behind");
    }

    #[test]
    fn a_save_leaves_no_temp_file_behind() {
        let (dir, store) = store();
        store.add(profile("a")).unwrap();
        store.add(profile("b")).unwrap();
        assert!(!dir.path().join("profiles.json.tmp").exists());
        assert_eq!(store.all().len(), 2);
    }

    /// Read from the environment, but only when it names an absolute directory:
    /// a relative XDG_CONFIG_HOME would put a credentials file somewhere that
    /// changes with the shell's working directory.
    ///
    /// Unix-only because the assertions are written in POSIX paths — `/xdg` is
    /// not absolute on Windows, which has no XDG convention to honour anyway.
    #[cfg(unix)]
    #[test]
    fn the_config_path_follows_xdg_only_when_it_is_absolute() {
        let home = Some(OsString::from("/home/u"));

        assert_eq!(
            ProfileStore::config_path(Some("/xdg".into()), home.clone(), "app"),
            PathBuf::from("/xdg/app/profiles.json")
        );
        assert_eq!(
            ProfileStore::config_path(None, home.clone(), "app"),
            PathBuf::from("/home/u/.config/app/profiles.json")
        );
        for bad in ["", "relative/path"] {
            assert_eq!(
                ProfileStore::config_path(Some(bad.into()), home.clone(), "app"),
                PathBuf::from("/home/u/.config/app/profiles.json"),
                "XDG_CONFIG_HOME={bad:?} should have been ignored"
            );
        }
    }

    #[test]
    fn a_corrupt_file_stops_a_write_instead_of_deleting_everything() {
        let (dir, store) = store();
        fs::write(dir.path().join("profiles.json"), "{ not json").unwrap();
        assert!(store.add(profile("a")).is_err());
        // And the bad file is still there, unmodified, for the user to fix.
        assert_eq!(
            fs::read_to_string(dir.path().join("profiles.json")).unwrap(),
            "{ not json"
        );
    }
}
