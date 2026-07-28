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

        Req::Delete(ids) => delete_many(client, ids),
    }
}

/// Delete each id, and report what actually happened to each one.
///
/// A partial failure is the normal case in bulk. Reporting only "done" would
/// leave rows the user believes are gone — and reporting only "failed" would
/// hide the ones that really were deleted.
fn delete_many(client: &ApiClient, ids: Vec<String>) -> Resp {
    let total = ids.len();
    let failed: Vec<String> = ids
        .iter()
        .filter_map(|id| {
            resource::delete(client, id)
                .err()
                .map(|e| format!("{id}: {e}"))
        })
        .collect();

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
