//! The network, on other threads, so the UI never freezes.
//!
//! Two lanes, both feeding one response channel:
//!
//! - `user` — what the operator just pressed. Never made to wait behind a refresh.
//! - `poll` — the periodic reload. Its own lane so a two-second tick can't queue
//!   ahead of (or behind) a delete the user is watching for.
//!
//! The lanes are the whole reason the UI stays responsive under a slow API, and
//! they are why `Req`/`Resp` are enums rather than callbacks: a message can be
//! sent from anywhere and answered on whichever lane makes sense.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use serde_json::Value;

use crate::client::{gave_up_waiting, ApiClient};
use crate::resource;

pub(super) enum Req {
    Items,
    Detail(String),
    /// `name` is carried alongside the body only so the status line can say what
    /// was created without having to guess which field of the body is the name.
    Create {
        name: String,
        body: Value,
    },
    /// Change one object. The body carries only the fields that differ — see
    /// `resource::edit_body`.
    ///
    /// Not a bulk, unlike Delete: "delete these twelve" is one intention, while
    /// "set these twelve to the same name" is not an operation anyone wants.
    Update {
        id: String,
        body: Value,
    },
    /// One id or many. Bulk is a client-side fan-out over the single-item call,
    /// so each target can fail on its own and be reported on its own.
    Delete(Vec<String>),
}

pub(super) enum Resp {
    Items(Vec<Value>),
    Detail(String, Value),
    /// An operation finished, well or badly.
    ///
    /// "Failed" and "stale" are separate questions, which is the point of this
    /// shape: a bulk delete where 2 of 3 succeeded is BOTH an error to report and
    /// a list to reload — collapsing them would leave rows on screen that the
    /// server no longer has.
    Done {
        message: String,
        error: bool,
        reload: bool,
    },
}

impl Resp {
    fn ok(message: impl Into<String>, reload: bool) -> Self {
        Resp::Done {
            message: message.into(),
            error: false,
            reload,
        }
    }
    fn err(message: impl Into<String>, reload: bool) -> Self {
        Resp::Done {
            message: message.into(),
            error: true,
            reload,
        }
    }
}

pub(super) struct Workers {
    pub(super) user: Sender<Req>,
    pub(super) poll: Sender<Req>,
    pub(super) resp: Receiver<Resp>,
    /// How many requests are in flight on the user lane. The App decides what to
    /// draw; the worker knows what is running — one shared counter joins them.
    pub(super) busy: Arc<AtomicUsize>,
}

pub(super) fn spawn_workers(client: ApiClient) -> Workers {
    let (resp_tx, resp) = mpsc::channel();
    let busy = Arc::new(AtomicUsize::new(0));
    Workers {
        user: lane(client.clone(), resp_tx.clone(), Some(busy.clone())),
        // The poll lane does NOT count as busy: a background refresh must not
        // spin the spinner, or the tool looks permanently mid-operation.
        poll: lane(client, resp_tx, None),
        resp,
        busy,
    }
}

fn lane(client: ApiClient, resp: Sender<Resp>, busy: Option<Arc<AtomicUsize>>) -> Sender<Req> {
    let (tx, rx) = mpsc::channel::<Req>();
    thread::spawn(move || {
        for req in rx {
            if let Some(b) = &busy {
                b.fetch_add(1, Ordering::Relaxed);
            }
            let out = handle(&client, req);
            if let Some(b) = &busy {
                b.fetch_sub(1, Ordering::Relaxed);
            }
            // A send failure means the UI is gone; there is nobody left to tell.
            if resp.send(out).is_err() {
                return;
            }
        }
    });
    tx
}

fn handle(client: &ApiClient, req: Req) -> Resp {
    match req {
        Req::Items => match resource::list(client) {
            Ok(items) => Resp::Items(items),
            Err(e) => Resp::err(format!("Load failed: {e}"), false),
        },

        Req::Detail(id) => match resource::get(client, &id) {
            Ok(v) => Resp::Detail(id, v),
            Err(e) => Resp::err(format!("Cannot open {id}: {e}"), false),
        },

        Req::Create { name, body } => match resource::create(client, body) {
            Ok(_) => Resp::ok(format!("Created '{name}'"), true),
            // A timeout is not a verdict: the server may well have created it.
            // Calling that "failed" invites the user to create a duplicate.
            Err(e) if gave_up_waiting(&e) => Resp::ok(
                format!("'{name}': stopped waiting for an answer — check the list"),
                true,
            ),
            Err(e) => Resp::err(format!("Create failed: {e}"), false),
        },

        Req::Update { id, body } => match resource::update(client, &id, body) {
            Ok(_) => Resp::ok(format!("Updated {id}"), true),
            // Same reasoning as Create: the change may well have landed, and
            // "failed" would invite the user to make it twice.
            Err(e) if gave_up_waiting(&e) => Resp::ok(
                format!("{id}: stopped waiting for an answer — check the row"),
                true,
            ),
            Err(e) => Resp::err(format!("Update failed: {e}"), false),
        },

        Req::Delete(ids) => delete_many(client, ids),
    }
}

/// How many deletes are in flight at once.
///
/// Bounded rather than one thread per id: a bulk of 200 would open 200
/// connections at a host that is usually the one thing everything else depends
/// on. Eight hides the latency without looking like an attack.
const BULK_CONCURRENCY: usize = 8;

/// Delete each id, and report what actually happened to each one.
///
/// A partial failure is the normal case in bulk. Reporting only "done" would
/// leave rows the user believes are gone — and reporting only "failed" would
/// hide the ones that really were deleted.
///
/// Run in parallel because these are independent round trips: sequentially, 20
/// deletes against a slow host cost 20× the latency for no reason. Scoped
/// threads, not an async runtime — the client is already blocking, and borrowing
/// it here needs nothing more.
fn delete_many(client: &ApiClient, ids: Vec<String>) -> Resp {
    let total = ids.len();
    let mut failed: Vec<String> = Vec::new();

    // ponytail: chunked, so each round waits for its slowest member. A shared
    // work queue would keep every slot busy — worth it only once bulks are large
    // AND per-item time is uneven.
    for chunk in ids.chunks(BULK_CONCURRENCY) {
        std::thread::scope(|scope| {
            let running: Vec<_> = chunk
                .iter()
                .map(|id| {
                    scope.spawn(move || {
                        resource::delete(client, id)
                            .err()
                            .map(|e| format!("{id}: {e}"))
                    })
                })
                .collect();
            for handle in running {
                match handle.join() {
                    Ok(Some(why)) => failed.push(why),
                    Ok(None) => {}
                    // A panicked worker must not disappear from the count: the
                    // user would be told the bulk was smaller than it was.
                    Err(_) => failed.push("a delete panicked".into()),
                }
            }
        });
    }

    match (failed.len(), total) {
        (0, 1) => Resp::ok("Deleted", true),
        (0, n) => Resp::ok(format!("Deleted {n}"), true),
        // Still a reload: whatever DID succeed is gone from the server.
        (n, total) => Resp::err(
            format!("{n} of {total} failed — {}", failed.join("; ")),
            true,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    /// A host that refuses exactly one of the ids.
    fn server_refusing(id: &str) -> MockServer {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(DELETE).path(format!("/items/{id}"));
            then.status(403)
                .json_body(serde_json::json!({ "message": "Forbidden" }));
        });
        server.mock(|when, then| {
            when.method(DELETE);
            then.status(200)
                .json_body(serde_json::json!({ "ok": true }));
        });
        server
    }

    #[test]
    fn a_partial_bulk_names_what_failed_and_still_asks_for_a_reload() {
        // The rows that DID delete are gone from the server, so a screen that
        // isn't reloaded is showing things that no longer exist.
        let server = server_refusing("i-2");
        let client = ApiClient::new(&server.base_url(), "t");
        let ids = ["i-1", "i-2", "i-3"].map(String::from).to_vec();

        let Resp::Done {
            message,
            error,
            reload,
        } = delete_many(&client, ids)
        else {
            panic!("a bulk delete always answers with Done");
        };
        assert!(error);
        assert!(reload, "two of them really are gone");
        assert!(message.starts_with("1 of 3 failed"), "{message}");
        assert!(
            message.contains("i-2"),
            "the user needs to know WHICH: {message}"
        );
    }

    #[test]
    fn every_id_is_attempted_even_past_the_concurrency_limit() {
        // Chunking is an implementation detail; a bulk larger than one chunk
        // must still delete every single id.
        let server = MockServer::start();
        let all = server.mock(|when, then| {
            when.method(DELETE);
            then.status(200)
                .json_body(serde_json::json!({ "ok": true }));
        });
        let client = ApiClient::new(&server.base_url(), "t");
        let n = BULK_CONCURRENCY * 2 + 3;
        let ids: Vec<String> = (0..n).map(|i| format!("i-{i}")).collect();

        let Resp::Done { message, error, .. } = delete_many(&client, ids) else {
            panic!("a bulk delete always answers with Done");
        };
        assert!(!error, "{message}");
        assert_eq!(message, format!("Deleted {n}"));
        all.assert_calls(n);
    }
}
