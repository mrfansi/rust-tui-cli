//! Behaviour tests for the TUI: keypress in, state out.
//!
//! No terminal is involved. The App is the whole decision layer, so driving it
//! with real KeyEvents tests what the user actually gets — and none of it needs
//! a server, a screen, or a sleep.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

use super::app::{App, Screen};
use super::status_should_fade;
use super::worker::{Req, Resp};

fn app(items: &[(&str, &str)]) -> (App, Sender<Req>, Receiver<Req>) {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new("prod".into(), vec![("prod".into(), "https://x".into())]);
    app.screen = Screen::Items;
    let items = items
        .iter()
        .map(|(id, status)| json!({ "id": id, "name": id, "status": status, "owner": "ops" }))
        .collect::<Vec<_>>();
    app.handle(Resp::Items(items), &tx);
    // The reload triggered by handle() is not part of what these tests assert.
    while rx.try_recv().is_ok() {}
    (app, tx, rx)
}

fn press(app: &mut App, tx: &Sender<Req>, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE), tx);
}

fn typed(app: &mut App, tx: &Sender<Req>, text: &str) {
    for c in text.chars() {
        press(app, tx, KeyCode::Char(c));
    }
}

#[test]
fn a_filter_narrows_the_table_and_the_cursor_follows_it() {
    let (mut app, tx, _rx) = app(&[("web", "active"), ("db", "failed"), ("cache", "active")]);
    press(&mut app, &tx, KeyCode::End);
    assert_eq!(app.items_row.selected(), Some(2));

    press(&mut app, &tx, KeyCode::Char('/'));
    typed(&mut app, &tx, "db");
    assert_eq!(app.shown().len(), 1);
    // The cursor pointed at row 2 of a list that now has one row. Left alone it
    // selects nothing, and the next action reads the wrong row — or no row.
    assert_eq!(app.items_row.selected(), Some(0));
    assert_eq!(app.selected_id().as_deref(), Some("db"));
}

#[test]
fn esc_clears_the_filter_rather_than_leaving_it_running_invisibly() {
    let (mut app, tx, _rx) = app(&[("web", "active"), ("db", "failed")]);
    press(&mut app, &tx, KeyCode::Char('/'));
    typed(&mut app, &tx, "db");
    press(&mut app, &tx, KeyCode::Esc);
    assert!(!app.filtering);
    assert!(
        app.filter.is_empty(),
        "a hidden filter is a table lying about its size"
    );
    assert_eq!(app.shown().len(), 2);
}

#[test]
fn q_typed_into_a_filter_is_a_letter_not_a_quit() {
    let (mut app, tx, _rx) = app(&[("q-service", "active")]);
    press(&mut app, &tx, KeyCode::Char('/'));
    typed(&mut app, &tx, "q");
    assert!(!app.quit);
    assert_eq!(app.filter, "q");
}

#[test]
fn mark_all_applies_to_what_the_filter_shows_and_toggles_off() {
    let (mut app, tx, _rx) = app(&[("web", "active"), ("db", "failed"), ("web-2", "active")]);
    press(&mut app, &tx, KeyCode::Char('/'));
    typed(&mut app, &tx, "web");
    press(&mut app, &tx, KeyCode::Enter); // leave the filter box, keep the filter
    press(&mut app, &tx, KeyCode::Char('V'));
    assert_eq!(app.marks.len(), 2, "only what is shown");
    assert!(!app.marks.contains("db"));

    press(&mut app, &tx, KeyCode::Char('V'));
    assert!(
        app.marks.is_empty(),
        "pressing it again clears, not re-marks"
    );
}

#[test]
fn a_delete_asks_first_and_sends_nothing_until_it_is_confirmed() {
    let (mut app, tx, rx) = app(&[("web", "active")]);
    press(&mut app, &tx, KeyCode::Char('x'));
    assert!(app.confirm.is_some());
    assert!(
        rx.try_recv().is_err(),
        "nothing may be sent before confirmation"
    );

    // Any key that isn't y/Enter cancels — this is the last thing between a
    // keystroke and a deletion.
    press(&mut app, &tx, KeyCode::Char('n'));
    assert!(app.confirm.is_none());
    assert!(rx.try_recv().is_err());

    press(&mut app, &tx, KeyCode::Char('x'));
    press(&mut app, &tx, KeyCode::Char('y'));
    assert!(matches!(rx.try_recv(), Ok(Req::Delete(ids)) if ids == ["web"]));
}

#[test]
fn a_bulk_delete_targets_the_marks_not_the_row_under_the_cursor() {
    let (mut app, tx, rx) = app(&[("web", "active"), ("db", "failed"), ("cache", "active")]);
    press(&mut app, &tx, KeyCode::Char('v')); // mark web
    press(&mut app, &tx, KeyCode::Down);
    press(&mut app, &tx, KeyCode::Char('v')); // mark db
    press(&mut app, &tx, KeyCode::Down); // cursor now on cache, which is NOT marked
    press(&mut app, &tx, KeyCode::Char('x'));
    press(&mut app, &tx, KeyCode::Char('y'));

    let Ok(Req::Delete(mut ids)) = rx.try_recv() else {
        panic!("a delete must have been sent");
    };
    ids.sort();
    assert_eq!(ids, ["db", "web"]);
}

#[test]
fn a_mark_on_a_row_the_server_no_longer_has_is_dropped() {
    // Otherwise the next bulk action silently widens to an id that is gone.
    let (mut app, tx, _rx) = app(&[("web", "active"), ("db", "failed")]);
    press(&mut app, &tx, KeyCode::Char('v'));
    press(&mut app, &tx, KeyCode::Down);
    press(&mut app, &tx, KeyCode::Char('v'));
    assert_eq!(app.marks.len(), 2);

    app.handle(Resp::Items(vec![json!({ "id": "web" })]), &tx);
    assert_eq!(app.marks.len(), 1);
    assert!(app.marks.contains("web"));
}

#[test]
fn the_form_keeps_what_was_typed_when_it_is_refused() {
    let (mut app, tx, rx) = app(&[]);
    press(&mut app, &tx, KeyCode::Char('n'));
    press(&mut app, &tx, KeyCode::Enter); // Name is empty

    let form = app.form.as_ref().expect("the form must stay open");
    assert_eq!(form.error.as_deref(), Some("Name is required"));
    assert!(rx.try_recv().is_err());

    typed(&mut app, &tx, "web");
    press(&mut app, &tx, KeyCode::Enter);
    assert!(app.form.is_none());
    assert!(matches!(rx.try_recv(), Ok(Req::Create { name, .. }) if name == "web"));
}

#[test]
fn esc_closes_one_overlay_at_a_time() {
    let (mut app, tx, _rx) = app(&[("web", "active")]);
    press(&mut app, &tx, KeyCode::Char(' ')); // the actions menu
    assert!(app.menu.is_some());
    press(&mut app, &tx, KeyCode::Esc);
    assert!(app.menu.is_none());
    assert!(!app.quit, "Esc on a menu must not leave the tool");
}

#[test]
fn a_menu_action_runs_and_the_menu_gets_out_of_its_way() {
    let (mut app, tx, _rx) = app(&[("web", "active")]);
    press(&mut app, &tx, KeyCode::Char(' '));
    press(&mut app, &tx, KeyCode::Down); // "New item…"
    press(&mut app, &tx, KeyCode::Enter);
    assert!(
        app.menu.is_none(),
        "the form must not open underneath the menu"
    );
    assert!(app.form.is_some());
}

#[test]
fn switching_profile_is_a_request_the_event_loop_fulfils() {
    // The drawing half never sees a token, so it cannot build a client itself.
    let (mut app, tx, _rx) = app(&[]);
    app.profiles.push(("staging".into(), "https://s".into()));
    press(&mut app, &tx, KeyCode::Char('s'));
    press(&mut app, &tx, KeyCode::Down);
    press(&mut app, &tx, KeyCode::Enter);
    assert_eq!(app.switch_to.as_deref(), Some("staging"));
    assert!(app.picker.is_none());
}

#[test]
fn adding_a_profile_is_handed_to_the_event_loop_and_the_name_is_checked_first() {
    let (mut app, tx, _rx) = app(&[]);
    press(&mut app, &tx, KeyCode::Char('s'));
    press(&mut app, &tx, KeyCode::Char('a'));
    assert!(app.picker.is_none(), "the form replaces the picker");

    typed(&mut app, &tx, "Prod"); // capital letter: not a valid profile name
    press(&mut app, &tx, KeyCode::Tab);
    typed(&mut app, &tx, "x"); // the URL field arrives prefilled with "https://"
    press(&mut app, &tx, KeyCode::Tab);
    typed(&mut app, &tx, "secret-token");
    press(&mut app, &tx, KeyCode::Enter);

    let form = app.form.as_ref().expect("a refused form stays open");
    assert!(
        form.error.is_some(),
        "the name must be rejected on the field"
    );
    assert!(app.add_profile.is_none());

    // Fix the name and it goes through — as a request, because only the event
    // loop may write a token to disk.
    press(&mut app, &tx, KeyCode::Up);
    press(&mut app, &tx, KeyCode::Up);
    for _ in 0..4 {
        press(&mut app, &tx, KeyCode::Backspace);
    }
    typed(&mut app, &tx, "prod-eu");
    press(&mut app, &tx, KeyCode::Enter);
    assert_eq!(
        app.add_profile,
        Some((
            "prod-eu".to_string(),
            "https://x".to_string(),
            "secret-token".to_string()
        ))
    );
}

#[test]
fn a_failure_never_fades_out_from_under_the_user() {
    let long = Duration::from_secs(60);
    assert!(status_should_fade("Created 'web'", long, 0));
    assert!(
        !status_should_fade("Ready", long, 0),
        "nothing to revert to"
    );
    assert!(
        !status_should_fade("Created 'web'", long, 1),
        "still working"
    );
    assert!(
        !status_should_fade("✗ Delete failed: [403] Forbidden", long, 0),
        "the only copy of this message the user gets"
    );
}

#[test]
fn tabs_step_in_both_directions_and_wrap() {
    let (mut app, tx, _rx) = app(&[]);
    app.screen = Screen::Dashboard;
    press(&mut app, &tx, KeyCode::Tab);
    assert_eq!(app.screen.index(), 1);
    press(&mut app, &tx, KeyCode::Tab);
    assert_eq!(app.screen.index(), 0, "wraps");
    press(&mut app, &tx, KeyCode::Char('2'));
    assert_eq!(app.screen.index(), 1);
    // A digit past the last tab must do nothing rather than panic.
    press(&mut app, &tx, KeyCode::Char('9'));
    assert_eq!(app.screen.index(), 1);
}

#[test]
fn ctrl_c_quits_even_mid_typing() {
    let (mut app, tx, _rx) = app(&[]);
    press(&mut app, &tx, KeyCode::Char('n'));
    app.on_key(
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &tx,
    );
    assert!(app.quit);
}

#[test]
fn opening_a_detail_puts_the_json_on_screen_and_esc_comes_back() {
    let (mut app, tx, _rx) = app(&[("web", "active")]);
    app.handle(Resp::Detail("web".into(), json!({ "id": "web" })), &tx);
    assert_eq!(app.screen, Screen::Viewer);
    let viewer = app.viewer.as_ref().unwrap();
    assert!(viewer.lines.iter().any(|l| l.contains("\"id\"")));

    press(&mut app, &tx, KeyCode::Esc);
    assert_eq!(app.screen, Screen::Items);
    assert!(app.viewer.is_none());
}

#[test]
fn a_partial_bulk_failure_is_reported_as_a_failure_and_still_reloads() {
    let (mut app, tx, rx) = app(&[("web", "active")]);
    app.handle(
        Resp::Done {
            message: "1 of 3 failed — db: [403] Forbidden".into(),
            error: true,
            reload: true,
        },
        &tx,
    );
    assert!(super::app::status_is_error(&app.status), "{}", app.status);
    assert!(
        matches!(rx.try_recv(), Ok(Req::Items)),
        "the two that succeeded really are gone — the list is stale"
    );
}
