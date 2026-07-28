//! A fake API, so the tool can be seen working before there is a real one.
//!
//! A fresh clone has no profile and no server, so `cargo run` can only print an
//! error — which tells a developer evaluating this template nothing about what
//! it does. Two terminals fixes that:
//!
//! ```text
//! cargo run --example fake_api     # prints the two lines to paste
//! # then, in another terminal, paste them and:
//! cargo run                        # the TUI, with data in it
//! ```
//!
//! It serves the shape `src/resource.rs` expects: a list, one item, a create and
//! a delete. Nothing here is used by the tool itself — `httpmock` is a
//! dev-dependency, so this costs a release build nothing.

use httpmock::prelude::*;
use serde_json::json;

fn main() {
    let server = MockServer::start();

    let items = json!([
        { "id": "i-1", "name": "web",      "status": "active",   "owner": "ops"  },
        { "id": "i-2", "name": "db",       "status": "failed",   "owner": "data" },
        { "id": "i-3", "name": "cache",    "status": "pending",  "owner": "ops"  },
        { "id": "i-4", "name": "worker",   "status": "active",   "owner": "jobs" },
        // A status resource.rs does not recognise, on purpose: it must render as
        // "unknown" rather than being painted a confident green.
        { "id": "i-5", "name": "scheduler", "status": "draining", "owner": "jobs" },
    ]);

    server.mock(|when, then| {
        when.method(GET).path("/items");
        then.status(200).json_body(items.clone());
    });

    // One item, for Enter on a row. Any id answers, so a freshly created one
    // opens too. A prefix rather than a regex, and it cannot collide with the
    // list above: that path has no trailing slash.
    server.mock(|when, then| {
        when.method(GET).path_prefix("/items/");
        then.status(200).json_body(json!({
            "id": "i-1",
            "name": "web",
            "status": "active",
            "owner": "ops",
            "created": "2026-01-01T00:00:00Z",
            "note": "Served by examples/fake_api.rs — not a real API.",
        }));
    });

    server.mock(|when, then| {
        when.method(POST).path("/items");
        then.status(201)
            .json_body(json!({ "id": "i-6", "status": "pending" }));
    });

    server.mock(|when, then| {
        when.method(DELETE).path_prefix("/items/");
        then.status(204);
    });

    let app = env!("CARGO_PKG_NAME");
    let prefix = app.to_uppercase().replace('-', "_");
    println!("Fake API on {}", server.base_url());
    println!();
    println!("Paste this into another terminal, then run `cargo run`:");
    println!();
    println!("  export {prefix}_URL={}", server.base_url());
    println!("  export {prefix}_TOKEN=not-a-real-token");
    println!();
    println!("The list reloads every 10s, so deletes and creates will appear to");
    println!("do nothing — this server always answers with the same five rows.");
    println!();
    println!("Ctrl-C to stop.");

    // The server lives on a thread of its own; parking keeps the process (and
    // therefore the server) alive until the developer stops it.
    loop {
        std::thread::park();
    }
}
