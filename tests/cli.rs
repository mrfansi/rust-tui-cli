//! The CLI surface, exercised by running the binary.
//!
//! The unit tests cover the functions behind the commands; nothing covered the
//! commands themselves — argument parsing, what reaches stdout, and whether a
//! failure exits non-zero — until this file. Those are the contract a script
//! wrapping this tool depends on.
//!
//! Every test points `XDG_CONFIG_HOME` at a temporary directory. A test that
//! read the developer's real profile store would pass or fail depending on
//! whose machine it ran on, and one that wrote to it would be worse.

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

const APP: &str = env!("CARGO_PKG_NAME");

fn env_prefix() -> String {
    APP.to_uppercase().replace('-', "_")
}

fn cli(config: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin(APP).unwrap();
    cmd.env("XDG_CONFIG_HOME", config.path());
    // Cleared, not just unset: a developer who exports these for real work would
    // otherwise see every profile-store test skip the store entirely.
    cmd.env_remove(format!("{}_URL", env_prefix()));
    cmd.env_remove(format!("{}_TOKEN", env_prefix()));
    cmd
}

#[test]
fn help_names_every_subcommand() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("profile"))
        .stdout(contains("item"))
        .stdout(contains("completions"));
}

/// With nothing configured, the error has to name BOTH ways to fix it. Naming
/// only `profile add` sends a CI user to write a credentials file onto a shared
/// runner when an environment variable would have done.
#[test]
fn with_nothing_configured_the_error_names_both_ways_to_fix_it() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["item", "list"])
        .assert()
        .failure()
        .stderr(contains("profile add"))
        .stderr(contains(format!("{}_TOKEN", env_prefix())));
}

/// Refused before any request is made: a typo in `--profile` must not silently
/// fall back to the default one and aim the command at the wrong host.
#[test]
fn an_unknown_profile_is_refused_and_points_at_the_list() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["--profile", "nope", "item", "list"])
        .assert()
        .failure()
        .stderr(contains("profile list"));
}

#[test]
fn an_empty_profile_list_is_not_an_error() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(contains("No profiles yet"));
}

/// Both are generated from the same clap definitions as `--help`, so they cannot
/// describe a CLI this build does not have — but only if they are generated at
/// all, which is what this checks.
#[test]
fn completions_and_the_man_page_are_produced() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(contains(APP));
    cli(&dir)
        .arg("man")
        .assert()
        .success()
        // The roff header macro. Anything shorter would pass on an empty page.
        .stdout(contains(".TH"));
}

/// End to end: the environment supplies the credentials, the request goes out,
/// and `--json` hands the script the API's own array rather than a table.
#[test]
fn credentials_from_the_environment_reach_a_real_request() {
    use httpmock::prelude::*;

    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/items");
        then.status(200)
            .json_body(serde_json::json!([{ "id": "i-1", "name": "web" }]));
    });

    let dir = TempDir::new().unwrap();
    let out = cli(&dir)
        .env(format!("{}_URL", env_prefix()), server.base_url())
        .env(format!("{}_TOKEN", env_prefix()), "t")
        .args(["--json", "item", "list"])
        .assert()
        .success();

    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("--json did not print JSON: {e}\n{stdout}"));
    assert_eq!(parsed[0]["id"], "i-1");
}

/// The update path, end to end: only the flag that was given reaches the wire.
#[test]
fn item_set_patches_only_what_was_asked_for() {
    use httpmock::prelude::*;

    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(PATCH)
            .path("/items/i-1")
            .json_body(serde_json::json!({ "owner": "data" }));
        then.status(200)
            .json_body(serde_json::json!({ "id": "i-1" }));
    });

    let dir = TempDir::new().unwrap();
    cli(&dir)
        .env(format!("{}_URL", env_prefix()), server.base_url())
        .env(format!("{}_TOKEN", env_prefix()), "t")
        .args(["item", "set", "i-1", "--owner", "data"])
        .assert()
        .success()
        .stdout(contains("Updated"));
    mock.assert();
}

/// `item set` with no flags must not reach the network at all: an empty PATCH
/// can only fail or do nothing, and the user meant to change something.
#[test]
fn item_set_with_nothing_to_set_is_refused_before_any_request() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        // A URL that nothing is listening on: if the command reaches the network
        // this fails with a connection error instead of the message under test.
        .env(format!("{}_URL", env_prefix()), "http://127.0.0.1:1")
        .env(format!("{}_TOKEN", env_prefix()), "t")
        .args(["item", "set", "i-1"])
        .assert()
        .failure()
        .stderr(contains("Nothing to change"));
}

/// Half a pair is not a credential: with a URL but no token the tool must fall
/// through to the profile store rather than send the default profile's token to
/// whatever host the variable names.
#[test]
fn a_url_without_a_token_is_ignored() {
    let dir = TempDir::new().unwrap();
    cli(&dir)
        .env(format!("{}_URL", env_prefix()), "http://127.0.0.1:1")
        .args(["item", "list"])
        .assert()
        .failure()
        .stderr(contains("profile add"));
}
