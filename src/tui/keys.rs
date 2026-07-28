//! Keypress → action. A second `impl App`, deliberately in its own file: this one
//! decides WHICH action, `app` defines what the action does.
//!
//! Order matters and is the whole design. Overlays are consulted first, topmost
//! first, so a key never means two things at once — a `q` typed into a filter box
//! is a letter, not "quit".

use std::sync::mpsc::Sender;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{App, Screen};
use super::form::FieldKind;
use super::table::move_table;
use super::worker::Req;

/// Is this keypress a character being typed, rather than a command?
///
/// Ctrl and Alt make it a command, and `KeyCode::Char` carries the letter either
/// way — so without this, Ctrl-U (the readline habit for "clear the line")
/// inserts a `u`, and every terminal shortcut a user reaches for leaves litter in
/// the field. Shift is NOT a command: it is how capitals are typed.
fn is_typing(key: KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

impl App {
    pub(super) fn on_key(&mut self, key: KeyEvent, tx: &Sender<Req>) {
        // Ctrl-C always quits, from anywhere, including mid-typing. It is the one
        // key a terminal user expects to work when everything else is confusing.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }

        if self.help {
            // Any key closes it. Help you have to learn how to leave is a trap.
            self.help = false;
            return;
        }
        if self.confirm.is_some() {
            return self.confirm_key(key, tx);
        }
        if self.form.is_some() {
            return self.form_key(key, tx);
        }
        if self.menu.is_some() {
            return self.menu_key(key, tx);
        }
        if self.picker.is_some() {
            return self.picker_key(key, tx);
        }
        if self.filtering {
            return self.filter_key(key);
        }
        self.screen_key(key, tx)
    }

    // ---------- Overlays ----------

    fn confirm_key(&mut self, key: KeyEvent, tx: &Sender<Req>) {
        match key.code {
            // `y` and Enter only — never a bare Space or any key, because this is
            // the last thing standing between a keystroke and a deletion.
            KeyCode::Char('y') | KeyCode::Enter => self.accept_confirm(tx),
            _ => {
                self.confirm = None;
                self.set_status("Cancelled");
            }
        }
    }

    fn form_key(&mut self, key: KeyEvent, tx: &Sender<Req>) {
        // The two keys that END the form are handled before the form is borrowed.
        match key.code {
            KeyCode::Esc => {
                self.form = None;
                self.set_status("Cancelled");
                return;
            }
            KeyCode::Enter => return self.submit_form(tx),
            _ => {}
        }
        let typing = is_typing(key);
        let Some(form) = &mut self.form else { return };
        match key.code {
            KeyCode::Tab | KeyCode::Down => form.move_focus(1),
            KeyCode::BackTab | KeyCode::Up => form.move_focus(-1),
            KeyCode::Left => form.cycle(-1),
            KeyCode::Right => form.cycle(1),
            KeyCode::Backspace => form.backspace(),
            // Space picks the next option in a choice, and is an ordinary space
            // everywhere else — a name with two words must stay typeable.
            KeyCode::Char(' ') if typing => match form.fields.get(form.focus).map(|f| &f.kind) {
                Some(FieldKind::Choice(_)) => form.cycle(1),
                _ => form.type_char(' '),
            },
            KeyCode::Char(c) if typing => form.type_char(c),
            _ => {}
        }
    }

    fn menu_key(&mut self, key: KeyEvent, tx: &Sender<Req>) {
        match key.code {
            KeyCode::Esc | KeyCode::Left => {
                self.menu = None;
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_menu(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_menu(1),
            KeyCode::Enter => {
                // The menu is closed BEFORE the action runs, so an action that
                // opens another overlay (a form, a confirmation) isn't buried
                // under the menu it came from.
                let Some(menu) = self.menu.take() else { return };
                let at = menu.state.selected().unwrap_or(0);
                if let Some(item) = menu.items.get(at) {
                    (item.run)(self, tx);
                }
            }
            _ => {}
        }
    }

    fn move_menu(&mut self, delta: isize) {
        let Some(menu) = &mut self.menu else { return };
        let len = menu.items.len() as isize;
        if len == 0 {
            return;
        }
        let at = menu.state.selected().unwrap_or(0) as isize;
        menu.state
            .select(Some((at + delta).rem_euclid(len) as usize));
    }

    fn picker_key(&mut self, key: KeyEvent, tx: &Sender<Req>) {
        let len = self.profiles.len() as isize;
        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Char('a') => self.open_profile_form(tx),
            KeyCode::Enter => {
                let at = self.picker.take().and_then(|s| s.selected()).unwrap_or(0);
                if let Some((name, _)) = self.profiles.get(at) {
                    // The event loop owns the store, so it is the only half that
                    // can turn a name into a client. This is the request.
                    self.switch_to = Some(name.clone());
                }
            }
            code if len > 0 => {
                let delta = match code {
                    KeyCode::Up | KeyCode::Char('k') => -1,
                    KeyCode::Down | KeyCode::Char('j') => 1,
                    _ => return,
                };
                if let Some(state) = &mut self.picker {
                    let at = state.selected().unwrap_or(0) as isize;
                    state.select(Some((at + delta).rem_euclid(len) as usize));
                }
            }
            _ => {}
        }
    }

    fn filter_key(&mut self, key: KeyEvent) {
        match key.code {
            // Esc clears the filter as well as leaving it: a filter you left
            // running while its box disappeared is a table lying about its size.
            KeyCode::Esc => {
                self.filter.clear();
                self.filtering = false;
            }
            KeyCode::Enter => self.filtering = false,
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(c) if is_typing(key) => self.filter.push(c),
            _ => {}
        }
        // Every keystroke changes what is shown, so the cursor must follow.
        let shown = self.shown().len();
        super::table::clamp(&mut self.items_row, shown);
    }

    // ---------- Screens ----------

    fn screen_key(&mut self, key: KeyEvent, tx: &Sender<Req>) {
        // Global keys first — they work the same on every screen, which is the
        // only reason they can be muscle memory.
        match key.code {
            KeyCode::Char('q') => {
                self.quit = true;
                return;
            }
            KeyCode::Char('?') => {
                self.help = true;
                return;
            }
            KeyCode::Char('s') => {
                self.open_picker();
                return;
            }
            KeyCode::Char('r') => {
                self.set_status("Refreshing…");
                self.reload(tx);
                return;
            }
            KeyCode::Tab | KeyCode::Right if self.screen != Screen::Viewer => {
                self.screen = self.screen.step(1);
                return;
            }
            KeyCode::BackTab | KeyCode::Left if self.screen != Screen::Viewer => {
                self.screen = self.screen.step(-1);
                return;
            }
            KeyCode::Char(c @ '1'..='9') => {
                let at = c as usize - '1' as usize;
                if let Some(&screen) = super::app::TAB_SCREENS.get(at) {
                    self.screen = screen;
                }
                return;
            }
            _ => {}
        }

        match self.screen {
            Screen::Dashboard => {}
            Screen::Items => self.items_key(key, tx),
            Screen::Viewer => self.viewer_key(key),
        }
    }

    fn items_key(&mut self, key: KeyEvent, tx: &Sender<Req>) {
        match key.code {
            KeyCode::Char('/') => {
                self.filtering = true;
                self.set_status("Filter: type to narrow, Enter to keep, Esc to clear");
            }
            KeyCode::Char('n') => self.open_new_form(tx),
            // The same method the menu's "Edit…" holds, never a second copy of
            // its body — that is what stops a keybinding and a menu item from
            // drifting into doing slightly different things.
            KeyCode::Char('e') => self.open_edit_form(tx),
            KeyCode::Char('x') => self.ask_delete(tx),
            KeyCode::Char('v') => self.toggle_mark(),
            KeyCode::Char('V') => self.mark_all_shown(),
            KeyCode::Char(' ') => self.open_item_menu(),
            KeyCode::Enter => self.open_detail(tx),
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                let shown = self.shown().len();
                super::table::clamp(&mut self.items_row, shown);
            }
            code => {
                let len = self.shown().len();
                move_table(&mut self.items_row, code, len);
            }
        }
    }

    fn viewer_key(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Backspace) {
            self.viewer = None;
            self.screen = Screen::Items;
            return;
        }
        let Some(viewer) = &mut self.viewer else {
            self.screen = Screen::Items;
            return;
        };
        // The last line stays reachable but the view can't be scrolled into
        // emptiness — a blank pane reads as "nothing here", not "past the end".
        let max = viewer.lines.len().saturating_sub(1) as u16;
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => viewer.scroll = (viewer.scroll + 1).min(max),
            KeyCode::Up | KeyCode::Char('k') => viewer.scroll = viewer.scroll.saturating_sub(1),
            KeyCode::PageDown => viewer.scroll = (viewer.scroll + 10).min(max),
            KeyCode::PageUp => viewer.scroll = viewer.scroll.saturating_sub(10),
            KeyCode::Home => viewer.scroll = 0,
            KeyCode::End => viewer.scroll = max,
            _ => {}
        }
    }
}
