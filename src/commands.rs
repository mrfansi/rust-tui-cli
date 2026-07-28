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

/// The environment variable prefix: the binary's name, upper-cased.
/// `rust-tui-cli` → `RUST_TUI_CLI_URL` / `RUST_TUI_CLI_TOKEN`.
fn env_prefix(app: &str) -> String {
    app.to_uppercase().replace('-', "_")
}

/// A client built from the environment, for CI — where writing a credentials
/// file onto a shared runner's disk is worse than passing it in.
///
/// Both variables or neither. A URL on its own would otherwise be sent the
/// DEFAULT PROFILE'S token: a production credential aimed at a staging host, or
/// the reverse, from a variable someone set expecting it to be ignored.
/// Empty counts as absent, because an unset secret in CI expands to `""` rather
/// than disappearing.
fn env_client(app: &str) -> Option<ApiClient> {
    let prefix = env_prefix(app);
    let var = |suffix: &str| std::env::var(format!("{prefix}_{suffix}")).ok();
    let (url, token) = credentials(var("URL"), var("TOKEN"))?;
    Some(ApiClient::new(&url, &token))
}

/// The pair rule on its own, so it can be tested without setting variables that
/// every other test in the process would also see.
fn credentials(url: Option<String>, token: Option<String>) -> Option<(String, String)> {
    let non_empty = |v: Option<String>| v.filter(|v| !v.is_empty());
    match (non_empty(url), non_empty(token)) {
        (Some(url), Some(token)) => Some((url, token)),
        _ => None,
    }
}

/// The client for `--profile <name>`, or from the environment, or for the
/// default profile — in that order.
pub fn resolve_client(store: &ProfileStore, profile: &Option<String>) -> Result<ApiClient> {
    // An explicit `--profile` beats the environment: a flag typed on this
    // command is a more specific statement of intent than a variable the shell
    // happens to be carrying, and silently ignoring it would send the operation
    // to the wrong host.
    if profile.is_none() {
        if let Some(client) = env_client(crate::APP_NAME) {
            return Ok(client);
        }
    }

    let p = match profile {
        Some(name) => store.get(name).ok_or_else(|| {
            anyhow!(
                "Profile '{name}' not found. See: {} profile list",
                crate::APP_NAME
            )
        })?,
        None => store.default().ok_or_else(|| {
            anyhow!(
                "No profiles yet. Run: {app} profile add\n\
                 Or set {prefix}_URL and {prefix}_TOKEN.",
                app = crate::APP_NAME,
                prefix = env_prefix(crate::APP_NAME),
            )
        })?,
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
    fn the_env_var_names_come_from_the_binary_name() {
        assert_eq!(env_prefix("rust-tui-cli"), "RUST_TUI_CLI");
        assert_eq!(env_prefix("flyctl"), "FLYCTL");
    }

    /// Half a credential is not a credential. A URL with no token would fall
    /// through to the default profile's token and aim it at another host.
    #[test]
    fn env_credentials_are_taken_only_as_a_complete_pair() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(
            credentials(s("https://api"), s("tok")),
            Some(("https://api".into(), "tok".into()))
        );
        assert_eq!(credentials(s("https://api"), None), None);
        assert_eq!(credentials(None, s("tok")), None);
        assert_eq!(credentials(None, None), None);
        // CI expands an unset secret to an empty string rather than dropping the
        // variable, so empty has to count as absent.
        assert_eq!(credentials(s("https://api"), s("")), None);
        assert_eq!(credentials(s(""), s("tok")), None);
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
