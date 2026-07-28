//! The profile store: every host this tool can talk to, and which one is default.
//!
//! Deliberately a plain JSON file rather than a config crate — it holds
//! credentials, so the only requirements are "readable by a human when something
//! goes wrong" and "not world-readable".

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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

    /// ~/.config/{binary name}/profiles.json
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        PathBuf::from(home)
            .join(".config")
            .join(crate::APP_NAME)
            .join("profiles.json")
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
        fs::write(&self.path, serde_json::to_string_pretty(&all)?)?;
        // This file holds tokens.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
        }
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
