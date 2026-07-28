//! The state, and everything that decides. `render` never decides; `keys` never
//! stores. Both go through here.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::{ListState, TableState};
use serde_json::Value;

use crate::filter::FilterMatcher;
use crate::resource;

use super::form::{Field, Form, FormKind};
use super::table::clamp;
use super::worker::Req;

// ---------- Screens ----------

#[derive(Debug, PartialEq, Clone, Copy)]
pub(super) enum Screen {
    Dashboard,
    Items,
    /// The detail of one item. Deliberately NOT a tab: it is the result of
    /// opening something, not a destination — as a tab it would be an empty box
    /// until the user arrived from Items.
    Viewer,
}

pub(super) const TABS: [&str; 2] = ["Dashboard", "Items"];
pub(super) const TAB_SCREENS: [Screen; 2] = [Screen::Dashboard, Screen::Items];

impl Screen {
    /// Which tab is highlighted. Viewer keeps Items lit, because that is where
    /// the user came from and where Esc will put them back.
    pub(super) fn index(self) -> usize {
        match self {
            Screen::Dashboard => 0,
            Screen::Items | Screen::Viewer => 1,
        }
    }

    pub(super) fn step(self, delta: isize) -> Self {
        let at = self.index() as isize;
        let n = TAB_SCREENS.len() as isize;
        TAB_SCREENS[(at + delta).rem_euclid(n) as usize]
    }
}

// ---------- Overlays ----------

/// A menu item's action. A closure with no captures becomes an `fn` on its own,
/// so the menu holds the action directly rather than simulating a keypress —
/// which is what makes the menu a SINGLE definition of what the action is.
pub(super) type MenuRun = fn(&mut App, &Sender<Req>);

pub(super) struct MenuItem {
    pub(super) label: String,
    pub(super) run: MenuRun,
}

impl MenuItem {
    fn new(label: impl Into<String>, run: MenuRun) -> Self {
        Self {
            label: label.into(),
            run,
        }
    }
}

pub(super) struct Menu {
    pub(super) items: Vec<MenuItem>,
    pub(super) state: ListState,
}

impl Menu {
    fn new(items: Vec<MenuItem>) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self { items, state }
    }
}

/// What a confirmation will do if it is accepted.
///
/// Carried as data rather than as a closure so the pending targets are visible in
/// the state — a confirmation whose subject you cannot inspect is one you cannot
/// render honestly ("Delete 12 items" must be able to name the 12).
pub(super) enum ConfirmAction {
    Delete(Vec<String>),
}

pub(super) struct Confirm {
    pub(super) prompt: String,
    pub(super) action: ConfirmAction,
}

pub(super) struct Viewer {
    pub(super) title: String,
    pub(super) lines: Vec<String>,
    pub(super) scroll: u16,
}

// ---------- App ----------

pub(super) struct App {
    pub(super) profile_name: String,
    /// (name, url) for the profile picker. The TOKENS stay in the event loop,
    /// which owns the store — the drawing half never sees a credential.
    pub(super) profiles: Vec<(String, String)>,
    /// Set when the user picks another profile; the event loop rebuilds the
    /// client, because it is the only half that can read a token.
    pub(super) switch_to: Option<String>,
    /// A profile the user just filled in. Saved by the event loop for the same
    /// reason: the store is a credentials file, and only that half holds it.
    pub(super) add_profile: Option<(String, String, String)>,

    pub(super) screen: Screen,
    pub(super) items: Vec<Value>,
    pub(super) items_row: TableState,
    pub(super) filter: String,
    pub(super) filtering: bool,
    /// Ids marked for a bulk action.
    pub(super) marks: HashSet<String>,

    pub(super) menu: Option<Menu>,
    pub(super) form: Option<Form>,
    pub(super) confirm: Option<Confirm>,
    pub(super) picker: Option<ListState>,
    pub(super) viewer: Option<Viewer>,
    pub(super) help: bool,

    pub(super) status: String,
    pub(super) busy: Arc<AtomicUsize>,
    /// A refresh is in flight; the next tick is skipped so rounds don't stack up
    /// when the API is slower than the refresh interval.
    pub(super) refresh_inflight: bool,
    pub(super) quit: bool,

    /// Where the table was drawn, so a click can be mapped to a row.
    pub(super) table_area: Rect,
    /// Each tab's horizontal span and the screen it leads to, recorded at render
    /// time. Computed there because that is the only place that knows how wide
    /// the labels came out.
    pub(super) tab_spans: Vec<(u16, u16, Screen)>,
    /// The row the tab labels sit on.
    pub(super) tab_row: u16,
    pub(super) spinner: usize,
    last_tick: Instant,
}

/// Is this status a failure?
///
/// The status line fades back to "Ready" after a few seconds — but a failure is
/// the ONLY copy of that message the user gets, so it must never fade. Marked by
/// a prefix rather than a second bool field so it survives being passed around as
/// a plain string.
pub(super) fn status_is_error(status: &str) -> bool {
    status.starts_with('✗')
}

impl App {
    pub(super) fn new(profile_name: String, profiles: Vec<(String, String)>) -> Self {
        Self {
            profile_name,
            profiles,
            switch_to: None,
            add_profile: None,
            screen: Screen::Dashboard,
            items: Vec::new(),
            items_row: TableState::default(),
            filter: String::new(),
            filtering: false,
            marks: HashSet::new(),
            menu: None,
            form: None,
            confirm: None,
            picker: None,
            viewer: None,
            help: false,
            status: "Ready".into(),
            busy: Arc::new(AtomicUsize::new(0)),
            refresh_inflight: false,
            quit: false,
            table_area: Rect::default(),
            tab_spans: Vec::new(),
            tab_row: 0,
            spinner: 0,
            last_tick: Instant::now(),
        }
    }

    pub(super) fn busy(&self) -> usize {
        self.busy.load(Ordering::Relaxed)
    }

    pub(super) fn animating(&self) -> bool {
        self.busy() > 0
    }

    pub(super) fn tick_anim(&mut self) {
        if self.animating() && self.last_tick.elapsed().as_millis() >= 120 {
            self.spinner = self.spinner.wrapping_add(1);
            self.last_tick = Instant::now();
        }
    }

    pub(super) fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
    }

    /// A failure, marked so it never fades out from under the user.
    pub(super) fn set_error(&mut self, msg: impl std::fmt::Display) {
        self.status = format!("✗ {msg}");
    }

    // ---------- Selection ----------

    /// Indices of the items the filter shows, in the API's own order.
    ///
    /// Matched against the DISPLAYED row, because searching for what you can see
    /// is the only behaviour that isn't surprising.
    /// ponytail: rebuilds the rows each call; if a list ever gets big enough for
    /// that to show, cache it and invalidate on items/filter change.
    pub(super) fn shown(&self) -> Vec<usize> {
        let matcher = FilterMatcher::new(&self.filter);
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| matcher.matches_any(resource::row(item).iter().map(String::as_str)))
            .map(|(i, _)| i)
            .collect()
    }

    pub(super) fn selected(&self) -> Option<&Value> {
        let at = self.items_row.selected()?;
        let idx = *self.shown().get(at)?;
        self.items.get(idx)
    }

    pub(super) fn selected_id(&self) -> Option<String> {
        self.selected().map(resource::id)
    }

    // ---------- Marks ----------

    pub(super) fn toggle_mark(&mut self) {
        if let Some(id) = self.selected_id() {
            if !self.marks.remove(&id) {
                self.marks.insert(id);
            }
            let n = self.marks.len();
            self.set_status(format!("{n} marked"));
        }
    }

    /// Mark everything the filter currently shows — the point of the filter.
    pub(super) fn mark_all_shown(&mut self) {
        let ids: Vec<String> = self
            .shown()
            .into_iter()
            .filter_map(|i| self.items.get(i).map(resource::id))
            .collect();
        // Toggle as a group: pressing it twice on the same filter clears it,
        // rather than leaving the user to unmark 200 rows one at a time.
        if ids.iter().all(|id| self.marks.contains(id)) {
            for id in &ids {
                self.marks.remove(id);
            }
        } else {
            self.marks.extend(ids);
        }
        let n = self.marks.len();
        self.set_status(format!("{n} marked"));
    }

    pub(super) fn clear_marks(&mut self) {
        self.marks.clear();
        self.set_status("Marks cleared");
    }

    /// What a bulk action applies to: the marks, or the selected row when there
    /// are none. Without the fallback, every action needs two implementations.
    pub(super) fn targets(&self) -> Vec<String> {
        if self.marks.is_empty() {
            self.selected_id().into_iter().collect()
        } else {
            self.marks.iter().cloned().collect()
        }
    }

    // ---------- Actions ----------

    pub(super) fn reload(&mut self, tx: &Sender<Req>) {
        self.refresh_inflight = true;
        let _ = tx.send(Req::Items);
    }

    pub(super) fn open_detail(&mut self, tx: &Sender<Req>) {
        match self.selected_id() {
            Some(id) => {
                self.set_status(format!("Opening {id}…"));
                let _ = tx.send(Req::Detail(id));
            }
            None => self.set_status("Nothing selected"),
        }
    }

    pub(super) fn ask_delete(&mut self, _tx: &Sender<Req>) {
        let ids = self.targets();
        match ids.len() {
            0 => self.set_status("Nothing selected"),
            1 => {
                self.confirm = Some(Confirm {
                    prompt: format!("Delete '{}'? This cannot be undone.", ids[0]),
                    action: ConfirmAction::Delete(ids),
                })
            }
            n => {
                self.confirm = Some(Confirm {
                    // Naming a couple of them is the difference between a
                    // confirmation and a rubber stamp.
                    prompt: format!(
                        "Delete {n} items ({}{})? This cannot be undone.",
                        ids.iter().take(2).cloned().collect::<Vec<_>>().join(", "),
                        if n > 2 { ", …" } else { "" }
                    ),
                    action: ConfirmAction::Delete(ids),
                })
            }
        }
    }

    pub(super) fn accept_confirm(&mut self, tx: &Sender<Req>) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        match confirm.action {
            ConfirmAction::Delete(ids) => {
                self.set_status(format!("Deleting {}…", ids.len()));
                self.marks.clear();
                let _ = tx.send(Req::Delete(ids));
            }
        }
    }

    pub(super) fn open_new_form(&mut self, _tx: &Sender<Req>) {
        self.form = Some(Form::new(
            FormKind::NewItem,
            "New item",
            vec![
                Field::text("Name", "").required(),
                Field::choice("Kind", vec!["app".into(), "db".into()]),
                // Shown only for a db, so one form covers both kinds instead of
                // asking for a field that means nothing half the time.
                Field::text("Image", "").required().when("Kind", "db"),
                Field::text("Owner", ""),
            ],
        ));
    }

    pub(super) fn open_profile_form(&mut self, _tx: &Sender<Req>) {
        self.picker = None;
        self.form = Some(Form::new(
            FormKind::AddProfile,
            "Add profile",
            vec![
                Field::text("Name", "").required(),
                Field::text("URL", "https://").required(),
                // Masked: a token typed on a shared screen (or into a recording)
                // is a token that has to be rotated.
                Field::secret("Token").required(),
            ],
        ));
    }

    /// Validate and send. Returns to the caller with the form still open when it
    /// is refused, so the user keeps what they typed.
    pub(super) fn submit_form(&mut self, tx: &Sender<Req>) {
        let Some(mut form) = self.form.take() else {
            return;
        };

        let refused = form.validate().err().or_else(|| {
            // Checked before the form closes, so a rejected name lands back on
            // the field the user can fix rather than in a status line.
            (form.kind == FormKind::AddProfile && !crate::commands::valid_name(&form.value("Name")))
                .then(|| "Name may only contain a-z, 0-9, - and _".to_string())
        });
        if let Some(why) = refused {
            form.error = Some(why);
            self.form = Some(form);
            return;
        }

        match form.kind {
            FormKind::AddProfile => {
                self.add_profile =
                    Some((form.value("Name"), form.value("URL"), form.value("Token")));
            }
            FormKind::NewItem => {
                let name = form.value("Name");
                // Read through `visible()`, not straight off the field: a hidden
                // field keeps whatever was typed before the choice changed, and
                // sending that would create an "app" carrying a database image
                // the user never asked for.
                let image = form
                    .visible()
                    .iter()
                    .any(|&i| form.fields[i].label == "Image")
                    .then(|| form.value("Image"));
                let body = resource::new_body(
                    &name,
                    &form.value("Kind"),
                    &form.value("Owner"),
                    image.as_deref(),
                );
                self.set_status(format!("Creating '{name}'…"));
                let _ = tx.send(Req::Create { name, body });
            }
        }
    }

    // ---------- The action catalogue ----------

    /// Every action on an item, defined ONCE. The Space menu is built from this,
    /// and so is the help — two parallel lists would drift, and help that lies is
    /// worse than no help.
    ///
    /// When this grows past a screenful, split it per group (submenus) into its
    /// own `actions.rs`, the way the reference implementation does.
    pub(super) fn open_item_menu(&mut self) {
        let marked = self.marks.len();
        let mut items = vec![
            MenuItem::new("Open detail", App::open_detail),
            MenuItem::new("New item…", App::open_new_form),
        ];
        items.push(MenuItem::new(
            if marked > 0 {
                format!("Delete {marked} marked…")
            } else {
                "Delete…".to_string()
            },
            App::ask_delete,
        ));
        if marked > 0 {
            items.push(MenuItem::new("Clear marks", |app, _| app.clear_marks()));
        }
        self.menu = Some(Menu::new(items));
    }

    pub(super) fn open_picker(&mut self) {
        let at = self
            .profiles
            .iter()
            .position(|(n, _)| *n == self.profile_name)
            .unwrap_or(0);
        let mut state = ListState::default();
        state.select(Some(at));
        self.picker = Some(state);
    }

    pub(super) fn overlay_open(&self) -> bool {
        self.help
            || self.confirm.is_some()
            || self.menu.is_some()
            || self.form.is_some()
            || self.picker.is_some()
    }

    // ---------- Mouse ----------

    pub(super) fn on_mouse(&mut self, m: MouseEvent, _tx: &Sender<Req>) {
        // While an overlay is up the mouse does nothing at all. A confirmation
        // that a stray click could answer is not a confirmation, and the click
        // that opened a menu is often followed by an accidental second one.
        if self.overlay_open() {
            return;
        }
        match m.kind {
            MouseEventKind::ScrollUp => self.scroll(-3),
            MouseEventKind::ScrollDown => self.scroll(3),
            MouseEventKind::Down(button) => self.click(button, m.column, m.row),
            _ => {}
        }
    }

    fn click(&mut self, button: MouseButton, column: u16, row: u16) {
        if button == MouseButton::Left {
            if let Some(screen) = self.tab_at(column, row) {
                self.screen = screen;
                return;
            }
        }
        // Selecting comes first for BOTH buttons, so the menu a right-click
        // opens is about the row under the pointer rather than about wherever
        // the keyboard cursor happened to be left.
        if self.select_row_at(row) && button == MouseButton::Right {
            self.open_item_menu();
        }
    }

    fn tab_at(&self, column: u16, row: u16) -> Option<Screen> {
        if row != self.tab_row {
            return None;
        }
        self.tab_spans
            .iter()
            .find(|(start, end, _)| (*start..*end).contains(&column))
            .map(|(_, _, screen)| *screen)
    }

    /// Select the row drawn at screen row `row`. Returns whether there was one.
    fn select_row_at(&mut self, row: u16) -> bool {
        if self.screen != Screen::Items {
            return false;
        }
        let area = self.table_area;
        // The first data row sits below the border AND the header; the last one
        // above the bottom border. A click on either border is not a row.
        let first = area.y + 2;
        if row < first || row + 1 >= area.y + area.height {
            return false;
        }
        // Add the table's own scroll offset, or every click past the first
        // screenful selects the wrong row.
        let at = (row - first) as usize + self.items_row.offset();
        if at >= self.shown().len() {
            return false;
        }
        self.items_row.select(Some(at));
        true
    }

    fn scroll(&mut self, delta: isize) {
        if let (Screen::Viewer, Some(viewer)) = (self.screen, &mut self.viewer) {
            let max = viewer.lines.len().saturating_sub(1) as u16;
            viewer.scroll = viewer.scroll.saturating_add_signed(delta as i16).min(max);
            return;
        }
        let len = self.shown().len();
        if len == 0 {
            return;
        }
        let at = self.items_row.selected().unwrap_or(0) as isize;
        self.items_row.select(Some(
            at.saturating_add(delta).clamp(0, len as isize - 1) as usize
        ));
    }

    // ---------- Responses ----------

    pub(super) fn handle(&mut self, resp: super::worker::Resp, tx: &Sender<Req>) {
        use super::worker::Resp;
        match resp {
            Resp::Items(items) => {
                self.items = items;
                self.refresh_inflight = false;
                // A mark on a row the server no longer has would silently widen
                // the next bulk action to an id that does not exist.
                let live: HashSet<String> = self.items.iter().map(resource::id).collect();
                self.marks.retain(|id| live.contains(id));
                let shown = self.shown().len();
                clamp(&mut self.items_row, shown);
            }

            Resp::Detail(id, value) => {
                self.viewer = Some(Viewer {
                    title: format!("Item {id}"),
                    lines: serde_json::to_string_pretty(&value)
                        .unwrap_or_else(|_| value.to_string())
                        .lines()
                        .map(str::to_string)
                        .collect(),
                    scroll: 0,
                });
                self.screen = Screen::Viewer;
                self.set_status("Ready");
            }

            Resp::Done {
                message,
                error,
                reload,
            } => {
                if error {
                    self.set_error(message);
                } else {
                    self.set_status(message);
                }
                if reload {
                    self.reload(tx);
                }
            }
        }
    }
}
