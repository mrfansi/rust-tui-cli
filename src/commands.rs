//! The CLI half: one function per subcommand.
//!
//! Each is thin on purpose — it resolves a client, calls the domain module, and
//! prints. Anything cleverer belongs in the domain module, where the TUI can
//! reach it too.

use anyhow::{anyhow, Result};
use dialoguer::{Confirm, Input, Password};

use crate::client::ApiClient;
use crate::config::{Profile, ProfileStore};
use crate::output::{json_output, print_json, table};
use crate::resource;

/// The client for `--profile <name>`, or for the default profile.
pub fn resolve_client(store: &ProfileStore, profile: &Option<String>) -> Result<ApiClient> {
    let p = match profile {
        Some(name) => store.get(name).ok_or_else(|| {
            anyhow!(
                "Profile '{name}' not found. See: {} profile list",
                crate::APP_NAME
            )
        })?,
        None => store
            .default()
            .ok_or_else(|| anyhow!("No profiles yet. Run: {} profile add", crate::APP_NAME))?,
    };
    Ok(ApiClient::new(&p.url, &p.token))
}

pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

// ---------- Profiles ----------

pub fn profile_add(
    store: &ProfileStore,
    name: Option<String>,
    url: Option<String>,
    token: Option<String>,
) -> Result<()> {
    // Prompting only for what wasn't given keeps the same command usable both
    // interactively and from a script.
    let name = match name {
        Some(n) => n,
        None => Input::new().with_prompt("Profile name").interact_text()?,
    };
    if !valid_name(&name) {
        return Err(anyhow!("Profile names may only contain a-z, 0-9, - and _"));
    }
    let url = match url {
        Some(u) => u,
        None => Input::new().with_prompt("API base URL").interact_text()?,
    };
    let token = match token {
        Some(t) => t,
        None => Password::new().with_prompt("API token").interact()?,
    };

    store.add(Profile {
        name: name.clone(),
        url,
        token,
        default: false,
    })?;
    println!("Added profile '{name}'.");
    Ok(())
}

pub fn profile_list(store: &ProfileStore) -> Result<()> {
    let profiles = store.try_all()?;
    if profiles.is_empty() {
        println!("No profiles yet. Run: {} profile add", crate::APP_NAME);
        return Ok(());
    }
    table(
        &["Name", "URL", "Default"],
        profiles
            .into_iter()
            .map(|p| vec![p.name, p.url, if p.default { "✓" } else { "" }.to_string()])
            .collect(),
    );
    Ok(())
}

/// Change a profile's URL or token in place — a host that moved, or a rotated
/// token. Remove-then-add would work, but it loses the default flag and asks the
/// user to retype the parts that did not change.
pub fn profile_set(
    store: &ProfileStore,
    name: &str,
    url: Option<String>,
    token: Option<String>,
) -> Result<()> {
    if url.is_none() && token.is_none() {
        return Err(anyhow!("Nothing to change. Pass --url and/or --token."));
    }
    store.update(name, url, token)?;
    println!("Updated profile '{name}'.");
    Ok(())
}

pub fn profile_use(store: &ProfileStore, name: &str) -> Result<()> {
    store.set_default(name)?;
    println!("Default profile is now '{name}'.");
    Ok(())
}

pub fn profile_remove(store: &ProfileStore, name: &str, yes: bool) -> Result<()> {
    if !yes
        && !Confirm::new()
            .with_prompt(format!("Remove profile '{name}' and its token?"))
            .default(false)
            .interact()?
    {
        println!("Cancelled.");
        return Ok(());
    }
    store.remove(name)?;
    println!("Removed profile '{name}'.");
    Ok(())
}

// ---------- Items (the demo domain) ----------

pub fn item_list(client: &ApiClient, filter: Option<String>) -> Result<()> {
    let items = resource::list(client)?;
    let items = match filter {
        Some(f) => resource::filtered(&items, &f),
        None => items,
    };
    if json_output() {
        print_json(&serde_json::Value::Array(items));
        return Ok(());
    }
    if items.is_empty() {
        println!("No items.");
        return Ok(());
    }
    table(
        &resource::HEADERS,
        items.iter().map(resource::row).collect(),
    );
    Ok(())
}

pub fn item_get(client: &ApiClient, id: &str) -> Result<()> {
    let item = resource::get(client, id)?;
    if json_output() {
        print_json(&item);
        return Ok(());
    }
    table(&resource::HEADERS, vec![resource::row(&item)]);
    Ok(())
}

pub fn item_create(
    client: &ApiClient,
    name: String,
    kind: String,
    owner: Option<String>,
    image: Option<String>,
) -> Result<()> {
    let body = resource::new_body(&name, &kind, &owner.unwrap_or_default(), image.as_deref());
    let created = resource::create(client, body)?;
    if json_output() {
        print_json(&created);
        return Ok(());
    }
    println!("Created '{name}'.");
    Ok(())
}

pub fn item_delete(client: &ApiClient, id: &str, yes: bool) -> Result<()> {
    // Destructive and irreversible: confirmed by default, skippable for scripts.
    if !yes
        && !Confirm::new()
            .with_prompt(format!("Delete item '{id}'? This cannot be undone."))
            .default(false)
            .interact()?
    {
        println!("Cancelled.");
        return Ok(());
    }
    resource::delete(client, id)?;
    println!("Deleted '{id}'.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_name_is_restricted_to_what_a_path_and_a_flag_can_carry() {
        assert!(valid_name("prod-eu"));
        assert!(valid_name("staging_2"));
        assert!(!valid_name(""));
        assert!(!valid_name("Prod"));
        assert!(!valid_name("prod eu"));
        assert!(!valid_name("../etc"));
    }

    #[test]
    fn an_unknown_profile_names_the_command_that_lists_them() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path().join("profiles.json"));
        let err = resolve_client(&store, &Some("nope".into())).unwrap_err();
        assert!(err.to_string().contains("profile list"), "{err}");

        // And with no profiles at all, the message must point at `profile add`
        // rather than at a list that is empty.
        let err = resolve_client(&store, &None).unwrap_err();
        assert!(err.to_string().contains("profile add"), "{err}");
    }
}
