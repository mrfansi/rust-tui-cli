//! Drawing, and nothing else. This file never decides anything: no request is
//! sent from here, no state is changed. Everything it needs was decided in `app`.

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Cell, Clear, List, ListItem, Paragraph, Row, Table, TableState, Wrap,
};

use crate::output::{field, first_line};
use crate::resource::{self, Health};

use super::app::{App, Screen, TABS};
use super::form::FieldKind;
use super::table::columns_that_fit;

/// One keybinding: the key, and what it does.
pub(super) struct Key(pub(super) &'static str, pub(super) &'static str);

/// Keys that work on every screen.
pub(super) const GLOBAL_KEYS: &[Key] = &[
    Key("1-9 / Tab / ←→", "switch tab"),
    Key("s", "profile list (Enter switch, a add)"),
    Key("r", "refresh"),
    Key("?", "this help"),
    Key("Esc", "cancel: close form / menu / filter"),
    Key("q / Ctrl-C", "quit"),
];

/// Keys for one screen.
///
/// The status bar shows the FIRST few entries of this same list, so it cannot
/// drift from the help. Two separate lists would eventually disagree, and help
/// that lies is worse than no help.
pub(super) fn screen_keys(screen: Screen) -> &'static [Key] {
    match screen {
        Screen::Dashboard => &[],
        Screen::Items => &[
            Key("/", "filter (text or regex)"),
            Key("Enter", "detail"),
            Key("Space", "menu"),
            Key("n", "new"),
            Key("v / V", "mark / mark all shown"),
            Key("x", "delete (marked, or this row)"),
            Key("↑↓", "select"),
        ],
        Screen::Viewer => &[Key("↑↓ / PgUp / PgDn", "scroll"), Key("Esc", "back")],
    }
}

const SPINNER: [&str; 4] = ["⠋", "⠙", "⠹", "⠸"];

pub(super) fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // tabs
        Constraint::Min(0),    // body
        Constraint::Length(1), // status
    ])
    .split(f.area());

    render_tabs(f, chunks[0], app);
    match app.screen {
        Screen::Dashboard => render_dashboard(f, chunks[1], app),
        Screen::Items => render_items(f, chunks[1], app),
        Screen::Viewer => render_viewer(f, chunks[1], app),
    }
    render_status(f, chunks[2], app);

    // Overlays, in the order they stack.
    if app.picker.is_some() {
        let rows: Vec<String> = app
            .profiles
            .iter()
            .map(|(name, url)| format!("{name}  —  {url}"))
            .collect();
        if let Some(state) = &mut app.picker {
            render_list_popup(f, "Profile — Enter switch · a add", &rows, state, 60, 50);
        }
    }
    if app.form.is_some() {
        render_form(f, app);
    }
    if let Some(menu) = &mut app.menu {
        let labels: Vec<String> = menu.items.iter().map(|i| i.label.clone()).collect();
        render_list_popup(f, "Actions", &labels, &mut menu.state, 44, 40);
    }
    if app.confirm.is_some() {
        render_confirm(f, app);
    }
    if app.help {
        render_help(f, app);
    }
}

// ---------- Chrome ----------

fn render_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = TABS
        .iter()
        .enumerate()
        .map(|(i, t)| Line::from(format!(" {}·{t} ", i + 1)))
        .collect();
    let tabs = ratatui::widgets::Tabs::new(titles)
        .select(app.screen.index())
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::bordered().title(Line::from(format!(" {} ", app.profile_name)).right_aligned()),
        );
    f.render_widget(tabs, area);
}

fn render_status(f: &mut Frame, area: Rect, app: &App) {
    let mut left = String::new();
    if app.busy() > 0 {
        left.push_str(SPINNER[app.spinner % SPINNER.len()]);
        left.push(' ');
    }
    if app.filtering || !app.filter.is_empty() {
        // While filtering, the filter IS the status: what is being typed matters
        // more than what happened six seconds ago.
        left.push_str(&format!(
            "/{}{}  ",
            app.filter,
            if app.filtering { "▏" } else { "" }
        ));
    }
    left.push_str(&app.status);

    // The right half is the same list the help shows, cut to what fits.
    let hints: String = screen_keys(app.screen)
        .iter()
        .take(4)
        .map(|Key(k, what)| format!("{k} {what}"))
        .collect::<Vec<_>>()
        .join("  ·  ");

    let style = if super::app::status_is_error(&app.status) {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let split =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);
    // Cut with an ellipsis rather than letting it run into the hints on the
    // right, where the two texts read as one corrupted sentence.
    let left = first_line(&left, split[0].width as usize);
    f.render_widget(Paragraph::new(Span::styled(left, style)), split[0]);
    f.render_widget(
        Paragraph::new(Span::styled(hints, Style::default().fg(Color::DarkGray)))
            .alignment(Alignment::Right),
        split[1],
    );
}

// ---------- Screens ----------

fn render_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let mut ok = 0;
    let mut warn = 0;
    let mut failed = 0;
    let mut unknown = 0;
    for item in &app.items {
        match resource::health(&field(item, "/status")) {
            Health::Ok => ok += 1,
            Health::Warning => warn += 1,
            Health::Failed => failed += 1,
            Health::Unknown => unknown += 1,
        }
    }

    let lines = vec![
        Line::from(Span::styled(
            format!("  Profile: {}", app.profile_name),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        count_line("Items", app.items.len(), Color::White),
        count_line("Healthy", ok, Color::Green),
        count_line("In progress", warn, Color::Yellow),
        count_line("Failed", failed, Color::Red),
        // Never green: an unrecognised status is not a healthy one, and a
        // confident colour on it would be a claim the tool cannot back.
        count_line("Unknown status", unknown, Color::DarkGray),
        Line::from(""),
        Line::from(Span::styled(
            "  Press 2 for the items table, ? for help.",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Overview ")),
        area,
    );
}

fn count_line(label: &str, n: usize, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {label:<16}"),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(n.to_string(), Style::default().fg(color)),
    ])
}

fn render_items(f: &mut Frame, area: Rect, app: &mut App) {
    // Recorded so a click can be mapped back to a row.
    app.table_area = area;

    let shown = app.shown();
    let rows: Vec<Vec<String>> = shown
        .iter()
        .filter_map(|&i| app.items.get(i))
        .map(|item| {
            let mut row = vec![if app.marks.contains(&resource::id(item)) {
                "✓".to_string()
            } else {
                String::new()
            }];
            row.extend(resource::row(item));
            row
        })
        .collect();

    // The count belongs in the title: a filtered table that doesn't say how much
    // it is hiding looks like the whole list.
    let title = if app.filter.is_empty() {
        format!(" Items ({}) ", app.items.len())
    } else {
        format!(" Items ({}/{}) ", rows.len(), app.items.len())
    };

    let headers: Vec<&str> = std::iter::once("")
        .chain(resource::HEADERS.iter().copied())
        .collect();
    let widths = [
        Constraint::Length(2),
        Constraint::Length(14),
        Constraint::Min(16),
        Constraint::Length(12),
        Constraint::Length(14),
    ];
    // Total width below which a column stops being worth its space. Whole
    // columns are dropped rather than every column being squeezed into a lie.
    let min_widths = [0, 0, 0, 46, 70];

    render_table(
        f,
        area,
        title,
        &headers,
        &widths,
        &min_widths,
        rows,
        &mut app.items_row,
        // Colour the status by what it MEANS, decided in the domain module —
        // the renderer must not learn the API's vocabulary.
        |col, text| {
            if col != 3 {
                return None;
            }
            match resource::health(text) {
                Health::Ok => Some(Style::default().fg(Color::Green)),
                Health::Warning => Some(Style::default().fg(Color::Yellow)),
                Health::Failed => Some(Style::default().fg(Color::Red)),
                Health::Unknown => None,
            }
        },
    );

    if app.items.is_empty() {
        // Drawn over the empty table: "nothing here" and "not loaded yet" look
        // identical otherwise, and only one of them is worth waiting for.
        let msg = if app.busy() > 0 {
            "Loading…"
        } else {
            "No items. Press n to create one, r to refresh."
        };
        let inner = area.inner(Margin::new(2, 2));
        f.render_widget(
            Paragraph::new(Span::styled(msg, Style::default().fg(Color::DarkGray))),
            inner,
        );
    }
}

fn render_viewer(f: &mut Frame, area: Rect, app: &App) {
    let Some(viewer) = &app.viewer else { return };
    f.render_widget(
        Paragraph::new(viewer.lines.join("\n"))
            .scroll((viewer.scroll, 0))
            .block(Block::bordered().title(format!(" {} ", viewer.title))),
        area,
    );
}

// ---------- Shared table ----------

/// Draw a table with a header, a highlighted row, and per-cell colours.
///
/// One helper rather than one table per screen: highlight style, header style,
/// truncation and column dropping are decisions that must be identical
/// everywhere, and copies of them do not stay identical.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_table(
    f: &mut Frame,
    area: Rect,
    title: String,
    headers: &[&str],
    widths: &[Constraint],
    min_widths: &[u16],
    rows: Vec<Vec<String>>,
    state: &mut TableState,
    cell_style: fn(usize, &str) -> Option<Style>,
) {
    let keep = columns_that_fit(min_widths, area.width);

    let header = Row::new(
        keep.iter()
            .filter_map(|&i| headers.get(i).copied())
            .collect::<Vec<_>>(),
    )
    .style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );

    let body: Vec<Row> = rows
        .into_iter()
        .map(|cells| {
            Row::new(
                keep.iter()
                    .filter_map(|&i| cells.get(i).map(|c| (i, c.clone())))
                    .map(|(i, c)| {
                        // Cut here, not at the pane edge: a clipped name reads as
                        // a complete, shorter one.
                        let c = first_line(&c, area.width.saturating_sub(4) as usize);
                        match cell_style(i, &c) {
                            Some(st) => Cell::from(c).style(st),
                            None => Cell::from(c),
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    let kept_widths: Vec<Constraint> = keep
        .iter()
        .filter_map(|&i| widths.get(i).copied())
        .collect();

    let table = Table::new(body, kept_widths)
        .header(header)
        .block(Block::bordered().title(title))
        // REVERSED rather than a background colour: a wide foreground tint turns
        // the selected row into a two-tone bar wherever a cell already has a
        // colour of its own.
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    f.render_stateful_widget(table, area, state);
}

// ---------- Overlays ----------

/// A box `pct_x`/`pct_y` of the screen, centred.
pub(super) fn centered(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - pct_y) / 2),
        Constraint::Percentage(pct_y),
        Constraint::Percentage((100 - pct_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - pct_x) / 2),
        Constraint::Percentage(pct_x),
        Constraint::Percentage((100 - pct_x) / 2),
    ])
    .split(v[1])[1]
}

fn render_list_popup(
    f: &mut Frame,
    title: &str,
    rows: &[String],
    state: &mut ratatui::widgets::ListState,
    pct_x: u16,
    pct_y: u16,
) {
    let area = centered(pct_x, pct_y, f.area());
    f.render_widget(Clear, area);
    let list = List::new(rows.iter().map(|r| ListItem::new(r.clone())))
        .block(Block::bordered().title(format!(" {title} ")))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("› ");
    f.render_stateful_widget(list, area, state);
}

fn render_form(f: &mut Frame, app: &App) {
    let Some(form) = &app.form else { return };
    let visible = form.visible();

    let mut lines: Vec<Line> = Vec::new();
    for i in visible {
        let field = &form.fields[i];
        let shown = match field.kind {
            FieldKind::Secret => "•".repeat(field.value.chars().count()),
            FieldKind::Choice(_) => format!("‹ {} ›", field.value),
            FieldKind::Text => field.value.clone(),
        };
        let focused = i == form.focus;
        // The whole label is padded as one unit, marker included. Padding only
        // the marker makes each row's value start at a column that depends on
        // how long its label happens to be.
        let name = format!("{}{}", field.label, if field.required { " *" } else { "" });
        let label = format!("  {}{name:<16}", if focused { "› " } else { "  " });
        lines.push(Line::from(vec![
            Span::styled(label, Style::default().fg(Color::DarkGray)),
            Span::styled(
                // A caret only where typing does something. On a choice it
                // would invite the user to type into a field that ignores them.
                if focused && field.kind.is_typed() {
                    format!("{shown}▏")
                } else {
                    shown
                },
                if focused {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Tab/↑↓ move · ←→/Space choose · Enter save · Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));

    // The error lives on the form's own border, not on the status line: the
    // status line fades, and an explanation that disappears while the user is
    // still looking for the field it names is worse than none.
    let mut block = Block::bordered().title(format!(" {} ", form.title));
    if let Some(err) = &form.error {
        block = block.title_bottom(Span::styled(
            format!(" {err} "),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }

    let area = centered(60, 50, f.area());
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_confirm(f: &mut Frame, app: &App) {
    let Some(confirm) = &app.confirm else { return };
    let area = centered(56, 26, f.area());
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(format!("  {}", confirm.prompt)),
            Line::from(""),
            Line::from(Span::styled(
                "  y / Enter to confirm · any other key cancels",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        // trim: false — trimming would eat the leading indent and shove the
        // prompt against the border.
        .wrap(Wrap { trim: false })
        .block(
            Block::bordered()
                .title(" Confirm ")
                .border_style(Style::default().fg(Color::Red)),
        ),
        area,
    );
}

fn render_help(f: &mut Frame, app: &App) {
    let mut lines = vec![Line::from(Span::styled(
        "  Anywhere",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.extend(GLOBAL_KEYS.iter().map(key_line));

    let screen_keys = screen_keys(app.screen);
    if !screen_keys.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {}", TABS[app.screen.index()]),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.extend(screen_keys.iter().map(key_line));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Any key closes this.",
        Style::default().fg(Color::DarkGray),
    )));

    let area = centered(64, 76, f.area());
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Help ")),
        area,
    );
}

fn key_line(Key(key, what): &Key) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {key:<18}"), Style::default().fg(Color::Cyan)),
        Span::raw(what.to_string()),
    ])
}
