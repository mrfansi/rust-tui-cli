//! THE FILE YOU REPLACE.
//!
//! One domain object ("item") and everything that is true about it: where it
//! lives on the API, how one row of it is displayed, and what its status means.
//! Nothing else in the tree knows any of that — `commands` prints whatever rows
//! it is handed, the TUI worker calls these functions, and `render` colours by
//! asking `health()`.
//!
//! To model your own domain: copy this file per object (`user.rs`, `deploy.rs`),
//! and see ARCHITECTURE.md for the five places a new object is wired in.

use anyhow::Result;
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::filter::FilterMatcher;
use crate::output::field;

/// Where this object lives. Every call below is built from it, so a change of
/// route is one line.
const PATH: &str = "/items";

pub const HEADERS: [&str; 4] = ["ID", "Name", "Status", "Owner"];

/// One row, in HEADERS order.
///
/// Built from the DISPLAYED values, because the filter matches what is on screen:
/// searching for what you can see is the only behaviour that isn't surprising.
pub fn row(item: &Value) -> Vec<String> {
    vec![
        field(item, "/id"),
        field(item, "/name"),
        field(item, "/status"),
        field(item, "/owner"),
    ]
}

/// The id used by `get`/`delete` and by the TUI's marks.
pub fn id(item: &Value) -> String {
    field(item, "/id")
}

/// What a status MEANS, so the UI can colour it without hard-coding the API's
/// vocabulary in the renderer.
///
/// Three states, not two: "unknown" exists because a value the API added last
/// week must not be painted green — a confident colour on an unrecognised status
/// is a claim the tool cannot back.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Health {
    Ok,
    Warning,
    Failed,
    Unknown,
}

pub fn health(status: &str) -> Health {
    match status {
        "active" | "running" | "ready" => Health::Ok,
        "pending" | "creating" | "updating" => Health::Warning,
        "failed" | "error" | "stopped" => Health::Failed,
        _ => Health::Unknown,
    }
}

/// Rows that pass the filter, keeping the API's own order.
pub fn filtered(items: &[Value], filter: &str) -> Vec<Value> {
    let matcher = FilterMatcher::new(filter);
    items
        .iter()
        .filter(|i| matcher.matches_any(row(i).iter().map(String::as_str)))
        .cloned()
        .collect()
}

// ---------- Calls ----------

/// A list endpoint answers with a bare array OR with the array under a key —
/// both are common enough that guessing wrong means an empty table against a
/// working API.
pub fn list(client: &ApiClient) -> Result<Vec<Value>> {
    let body = client.get(PATH)?;
    Ok(match body {
        Value::Array(items) => items,
        ref v => v
            .get("items")
            .or_else(|| v.get("data"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    })
}

pub fn get(client: &ApiClient, id: &str) -> Result<Value> {
    client.get(&format!("{PATH}/{id}"))
}

/// The body is built by the caller (a CLI flag set, or a TUI form) rather than
/// from a fixed argument list: a resource grows fields, and a signature that has
/// to grow with it makes every caller change for a field it doesn't set.
///
/// Given its own, longer timeout because creating something is the operation
/// that legitimately takes minutes (an image pull, a first build). The global
/// 30 s would cut the connection while the server kept working, and the user
/// would be told "failed" about something that succeeded.
pub fn create(client: &ApiClient, body: Value) -> Result<Value> {
    client.post(PATH, body, std::time::Duration::from_secs(120))
}

/// The shape `create` expects, from the parts every caller has.
pub fn new_body(name: &str, kind: &str, owner: &str, image: Option<&str>) -> Value {
    let mut body = json!({ "name": name, "kind": kind, "owner": owner });
    if let Some(image) = image {
        body["image"] = json!(image);
    }
    body
}

pub fn delete(client: &ApiClient, id: &str) -> Result<Value> {
    client.delete(&format!("{PATH}/{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    #[test]
    fn a_row_reads_in_header_order_and_never_panics_on_a_thin_object() {
        let full = json!({ "id": "i-1", "name": "web", "status": "active", "owner": "ops" });
        assert_eq!(row(&full), ["i-1", "web", "active", "ops"]);
        // An object missing everything is still a row of the right width — a
        // short row would shift every cell after it into the wrong column.
        assert_eq!(row(&json!({})).len(), HEADERS.len());
    }

    #[test]
    fn an_unrecognised_status_is_unknown_not_healthy() {
        assert_eq!(health("active"), Health::Ok);
        assert_eq!(health("failed"), Health::Failed);
        assert_eq!(health("degraded-in-some-new-way"), Health::Unknown);
    }

    #[test]
    fn the_filter_matches_any_visible_cell() {
        let items = vec![
            json!({ "id": "i-1", "name": "web", "status": "active", "owner": "ops" }),
            json!({ "id": "i-2", "name": "db", "status": "failed", "owner": "data" }),
        ];
        assert_eq!(filtered(&items, "failed").len(), 1);
        assert_eq!(filtered(&items, "ops")[0]["id"], "i-1");
        assert_eq!(filtered(&items, "").len(), 2);
    }

    #[test]
    fn a_list_is_read_whether_its_bare_or_wrapped() {
        for body in [
            json!([{ "id": "i-1" }]),
            json!({ "items": [{ "id": "i-1" }] }),
            json!({ "data": [{ "id": "i-1" }] }),
        ] {
            let server = MockServer::start();
            server.mock(|when, then| {
                when.method(GET).path("/items");
                then.status(200).json_body(body.clone());
            });
            let client = ApiClient::new(&server.base_url(), "t");
            assert_eq!(list(&client).unwrap().len(), 1, "shape: {body}");
        }
    }
}
