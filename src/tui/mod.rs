//! The TUI.
//!
//! Split along its data flow, not by type: `worker` talks to the network on
//! other threads and only knows Req/Resp; `app` holds the state and decides;
//! `keys` maps a keypress to a decision; `render` draws and decides nothing;
//! `form` and `table` are the vocabulary they share.
//!
//! `mod.rs` only ties them together — the terminal, the event loop, and the
//! profile store, which is the one thing the drawing half must never hold
//! (it contains tokens).

mod app;
mod form;
mod keys;
mod render;
mod table;
mod worker;

#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};

use crate::client::ApiClient;
use crate::config::ProfileStore;

use app::App;
use render::ui;
use worker::{spawn_workers, Req};

/// How often the list is reloaded in the background.
const REFRESH: Duration = Duration::from_secs(10);

/// How long a transient notice stays before the status line returns to "Ready".
const STATUS_IDLE: Duration = Duration::from_secs(6);

pub fn run(store: &ProfileStore, client: ApiClient, profile_name: String) -> Result<()> {
    let profiles: Vec<(String, String)> =
        store.all().into_iter().map(|p| (p.name, p.url)).collect();
    if profiles.is_empty() {
        println!("No profiles yet. Run: {} profile add", crate::APP_NAME);
        return Ok(());
    }

    let mut app = App::new(profile_name, profiles);
    let mut terminal = ratatui::init();
    set_mouse(true);
    restore_mouse_on_panic();
    let result = event_loop(&mut terminal, &mut app, store, client);
    // Restore the terminal even when the loop failed: a tool that leaves the
    // terminal in raw mode after an error has broken the shell it was run from.
    set_mouse(false);
    ratatui::restore();
    result
}

/// Capture mouse events (tab and row clicks, the wheel).
///
/// Side effect worth knowing about: while this is on, the terminal's own text
/// selection is disabled — most emulators fall back to Shift+drag for copying.
/// Turn mouse capture back off if we panic.
///
/// `ratatui::init()` already installs a hook, but it only leaves raw mode and
/// the alternate screen. Mouse capture was switched on *here*, so nothing else
/// switches it off — and a terminal left capturing the mouse answers every
/// click with an escape sequence until the user runs `reset`. That is the same
/// broken-shell-after-a-crash this file's error path is careful to avoid.
///
/// Installed AFTER `ratatui::init()` and chained rather than replacing: ours
/// disables the mouse, then ratatui's restores the terminal, then the default
/// one prints the panic. Replacing the hook would lose the other two.
fn restore_mouse_on_panic() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        set_mouse(false);
        previous(info);
    }));
}

fn set_mouse(on: bool) {
    let mut out = std::io::stdout();
    let _ = if on {
        ratatui::crossterm::execute!(out, EnableMouseCapture)
    } else {
        ratatui::crossterm::execute!(out, DisableMouseCapture)
    };
}

/// Should the status line revert to "Ready" now?
///
/// Tracked centrally rather than at every place that writes a status, so a notice
/// like "Created 'web'" doesn't linger forever. Two things never fade:
///
/// - "Ready" itself — there is nothing to revert to.
/// - A failure — it is the ONLY copy of that message the user gets. Erasing it
///   after six seconds loses it for good AND replaces it with a claim that
///   everything is fine.
pub(super) fn status_should_fade(status: &str, idle: Duration, busy: usize) -> bool {
    status != "Ready" && !app::status_is_error(status) && busy == 0 && idle >= STATUS_IDLE
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    store: &ProfileStore,
    client: ApiClient,
) -> Result<()> {
    let mut w = spawn_workers(client);
    app.busy = w.busy.clone();
    app.reload(&w.user);

    let mut last_refresh = Instant::now();
    let mut last_status = app.status.clone();
    let mut status_since = Instant::now();

    loop {
        while let Ok(resp) = w.resp.try_recv() {
            app.handle(resp, &w.user);
        }

        if app.status != last_status {
            last_status = app.status.clone();
            status_since = Instant::now();
        } else if status_should_fade(&app.status, status_since.elapsed(), app.busy()) {
            app.set_status("Ready");
            last_status = app.status.clone();
        }

        app.tick_anim();
        terminal.draw(|f| ui(f, app))?;

        // The in-flight guard stops refresh rounds stacking up when the API is
        // slower than the interval.
        if last_refresh.elapsed() >= REFRESH && !app.refresh_inflight {
            app.refresh_inflight = true;
            let _ = w.poll.send(Req::Items);
            last_refresh = Instant::now();
        }

        // Idle polling stays cheap; an animation needs a tighter loop to look
        // like an animation rather than a stutter.
        let poll = if app.animating() { 70 } else { 120 };
        if event::poll(Duration::from_millis(poll))? {
            match event::read()? {
                // Windows sends a Release for every Press; acting on both runs
                // every action twice.
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key, &w.user),
                Event::Mouse(m) => app.on_mouse(m, &w.user),
                _ => {}
            }
        }

        // Saving a profile happens here for the same reason switching does: the
        // store is a credentials file, and the drawing half never holds it. A
        // new profile is switched to straight away — nobody adds a host they
        // did not intend to use.
        if let Some((name, url, token)) = app.add_profile.take() {
            match store.add(crate::config::Profile {
                name: name.clone(),
                url: url.clone(),
                token,
                default: false,
            }) {
                Ok(()) => {
                    app.profiles.push((name.clone(), url));
                    app.switch_to = Some(name);
                }
                Err(e) => app.set_error(e),
            }
        }

        // Switching profile means a new client, which means new worker threads:
        // the old ones hold the old token and would keep answering with the wrong
        // host's data. Done here because this is the only half that can read a
        // token out of the store.
        if let Some(name) = app.switch_to.take() {
            match store.get(&name) {
                Some(p) => {
                    w = spawn_workers(ApiClient::new(&p.url, &p.token));
                    app.busy = w.busy.clone();
                    app.profile_name = name;
                    app.items.clear();
                    app.marks.clear();
                    app.set_status("Switched profile");
                    app.reload(&w.user);
                    last_refresh = Instant::now();
                }
                None => app.set_error(format!("Profile '{name}' is gone")),
            }
        }

        if app.quit {
            return Ok(());
        }
    }
}
