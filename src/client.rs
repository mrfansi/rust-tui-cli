//! The HTTP layer. One client, one place that knows how the API reports errors.
//!
//! Nothing above this file parses a status code or an error envelope: callers get
//! either a `Value` or an `ApiError` that already carries the status. Replace the
//! request builder to match your API (a different auth header, a tRPC-style POST
//! envelope, query signing) and every command and TUI worker follows.

use anyhow::{bail, Result};
use serde_json::Value;

/// A REST client for one profile (base URL + token).
///
/// Cloning shares the same reqwest connection pool, so it's cheap: each TUI
/// worker thread gets its own clone.
#[derive(Clone)]
pub struct ApiClient {
    url: String,
    token: String,
    http: reqwest::blocking::Client,
}

/// Written by hand, and it must stay that way: a derived Debug would print the
/// token into any `{:?}`, `unwrap()` panic or log line that ever touches a client.
impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiClient")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// An error the SERVER returned, with the status it returned it with.
///
/// Typed rather than a formatted string: callers need the status to tell a
/// refusal apart from a gateway giving up, and parsing it back out of a message
/// would break the moment the wording changed.
#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.status, self.message)
    }
}

impl std::error::Error for ApiError {}

/// Did this error mean "we stopped waiting", rather than "the server refused"?
///
/// Long operations keep running after the connection to them dies. Reporting a
/// timeout as "failed" tells the user the opposite of what happened, and the
/// obvious response is to run it again. A connection that was never established
/// is NOT this: nothing was dispatched, so that really is a failure.
pub fn gave_up_waiting(e: &anyhow::Error) -> bool {
    if e.downcast_ref::<reqwest::Error>()
        .is_some_and(|r| r.is_timeout())
    {
        return true;
    }
    // Gateway statuses come from something in front of the API giving up, not
    // from the API rejecting the request.
    matches!(
        e.downcast_ref::<ApiError>().map(|a| a.status),
        Some(502 | 503 | 504 | 524)
    )
}

impl ApiClient {
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            // A timeout is mandatory: without it one hanging request freezes the
            // TUI worker forever, and no other request can run.
            http: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        self.send(self.http.get(self.endpoint(path)), None)
    }

    /// POST, with the timeout stated by the caller.
    ///
    /// Explicit because "how long may this take" is a per-operation decision: a
    /// create that pulls an image legitimately runs for minutes, while the
    /// global 30 s is right for everything else. Raising the global timeout
    /// instead would force every other call to wait two minutes before it is
    /// allowed to report failure.
    pub fn post(&self, path: &str, body: Value, timeout: std::time::Duration) -> Result<Value> {
        self.send(
            self.http.post(self.endpoint(path)).json(&body),
            Some(timeout),
        )
    }

    pub fn delete(&self, path: &str) -> Result<Value> {
        self.send(self.http.delete(self.endpoint(path)), None)
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.url, path)
    }

    fn send(
        &self,
        req: reqwest::blocking::RequestBuilder,
        timeout: Option<std::time::Duration>,
    ) -> Result<Value> {
        let mut req = req.bearer_auth(&self.token);
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        let resp = req.send()?;
        let status = resp.status();

        if !status.is_success() {
            if status.as_u16() == 401 {
                bail!("Invalid or expired token (401).");
            }
            let body: Value = resp.json().unwrap_or(Value::Null);
            // The server's own words beat a generic status name ("Bad Request"),
            // which tells the user nothing they can act on. Several shapes are
            // tried because APIs disagree about where the message lives.
            let msg = ["/message", "/error", "/error/message", "/detail"]
                .iter()
                .find_map(|p| body.pointer(p).and_then(Value::as_str))
                .unwrap_or_else(|| status.canonical_reason().unwrap_or("error"));
            return Err(ApiError {
                status: status.as_u16(),
                message: msg.to_string(),
            }
            .into());
        }

        // A 204 has no body, and serde would call that a parse error.
        let text = resp.text()?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&text)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    #[test]
    fn get_sends_the_bearer_token_and_returns_the_body() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/items")
                .header("authorization", "Bearer tok123");
            then.status(200).json_body(json!([{ "id": "a" }]));
        });

        let client = ApiClient::new(&server.base_url(), "tok123");
        let out = client.get("/items").unwrap();

        mock.assert();
        assert_eq!(out, json!([{ "id": "a" }]));
    }

    #[test]
    fn a_refusal_keeps_the_servers_own_message() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST);
            then.status(400)
                .json_body(json!({ "message": "Name is taken" }));
        });

        let client = ApiClient::new(&server.base_url(), "t");
        let err = client
            .post("/items", json!({}), std::time::Duration::from_secs(5))
            .expect_err("400 must error");
        assert_eq!(err.to_string(), "[400] Name is taken");
        assert!(
            !gave_up_waiting(&err),
            "the server rejected this — it IS a failure"
        );
    }

    #[test]
    fn a_gateway_giving_up_is_not_the_server_refusing() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET);
            then.status(504).json_body(json!({}));
        });

        let client = ApiClient::new(&server.base_url(), "t");
        let err = client.get("/items").expect_err("504 must error");
        assert!(gave_up_waiting(&err), "504 is a proxy giving up: {err:#}");
    }

    #[test]
    fn a_timeout_is_not_a_verdict_on_the_operation() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST);
            then.status(200)
                .delay(std::time::Duration::from_millis(600))
                .json_body(json!({}));
        });

        let client = ApiClient::new(&server.base_url(), "t");
        let err = client
            .post("/items", json!({}), std::time::Duration::from_millis(80))
            .expect_err("the short timeout must trip");
        assert!(gave_up_waiting(&err), "{err:#}");
    }

    #[test]
    fn an_empty_body_is_null_not_a_parse_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(DELETE);
            then.status(204);
        });

        let client = ApiClient::new(&server.base_url(), "t");
        assert_eq!(client.delete("/items/a").unwrap(), Value::Null);
    }

    #[test]
    fn maps_401_to_a_friendly_message() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET);
            then.status(401)
                .json_body(json!({ "message": "Unauthorized" }));
        });

        let client = ApiClient::new(&server.base_url(), "t");
        let err = client.get("/items").unwrap_err();
        assert!(
            err.to_string().contains("Invalid or expired token"),
            "{err}"
        );
    }
}
