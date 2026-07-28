//! Table navigation and column fitting — the parts a table needs that ratatui
//! doesn't decide for you. Shared by every screen so navigation feels the same
//! everywhere.

use ratatui::crossterm::event::KeyCode;
use ratatui::widgets::TableState;

/// Keep the selection inside a list that just got shorter (a delete, a filter),
/// and select the first row when there is one and nothing is selected yet.
///
/// Without this the cursor points past the end, the table renders nothing
/// highlighted, and the next action reads the wrong row — or no row at all.
pub(super) fn clamp(state: &mut TableState, len: usize) {
    match (state.selected(), len) {
        (_, 0) => state.select(None),
        (Some(i), len) if i >= len => state.select(Some(len - 1)),
        (None, _) => state.select(Some(0)),
        _ => {}
    }
}

/// Arrows/jk, PgUp/PgDn, Home/End.
pub(super) fn move_table(state: &mut TableState, code: KeyCode, len: usize) {
    if len == 0 {
        return;
    }
    let delta: isize = match code {
        KeyCode::Down | KeyCode::Char('j') => 1,
        KeyCode::Up | KeyCode::Char('k') => -1,
        KeyCode::PageDown => 10,
        KeyCode::PageUp => -10,
        KeyCode::Home => -(len as isize),
        KeyCode::End => len as isize,
        _ => return,
    };
    let cur = state.selected().unwrap_or(0) as isize;
    state.select(Some(
        cur.saturating_add(delta).clamp(0, len as isize - 1) as usize
    ));
}

/// Which columns fit, given the total width each one needs to be worth showing.
///
/// Squeezed below what its columns need, ratatui shrinks EVERY column
/// proportionally, and the result is not merely short — it is wrong: "199.9 GB /
/// 784.9 GB" comes out as "199.9 GB / 784", a total with no unit and off by three
/// orders of magnitude. So whole columns are dropped instead, and every value
/// still on screen is true.
///
/// Returns INDICES, so the column given up can be one in the middle — a history
/// table would rather lose "Duration" than "Age".
pub(super) fn columns_that_fit(min_widths: &[u16], area_width: u16) -> Vec<usize> {
    min_widths
        .iter()
        .enumerate()
        .filter(|(_, min)| area_width >= **min)
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_at(i: Option<usize>) -> TableState {
        let mut s = TableState::default();
        s.select(i);
        s
    }

    #[test]
    fn movement_stops_at_both_ends_instead_of_wrapping() {
        let mut s = state_at(Some(0));
        move_table(&mut s, KeyCode::Up, 3);
        assert_eq!(s.selected(), Some(0));
        move_table(&mut s, KeyCode::End, 3);
        assert_eq!(s.selected(), Some(2));
        move_table(&mut s, KeyCode::Down, 3);
        assert_eq!(s.selected(), Some(2));
        move_table(&mut s, KeyCode::Home, 3);
        assert_eq!(s.selected(), Some(0));
    }

    #[test]
    fn a_page_jump_past_the_end_lands_on_the_last_row() {
        let mut s = state_at(Some(0));
        move_table(&mut s, KeyCode::PageDown, 4);
        assert_eq!(s.selected(), Some(3));
    }

    #[test]
    fn an_empty_list_has_nothing_selected() {
        let mut s = state_at(Some(2));
        move_table(&mut s, KeyCode::Down, 0);
        assert_eq!(s.selected(), Some(2), "movement does nothing…");
        clamp(&mut s, 0);
        assert_eq!(s.selected(), None, "…but clamping clears it");
    }

    #[test]
    fn the_selection_follows_a_list_that_shrank() {
        // The bug this exists to stop: delete the last row, and the next action
        // reads a row that is no longer there.
        let mut s = state_at(Some(9));
        clamp(&mut s, 4);
        assert_eq!(s.selected(), Some(3));
    }

    #[test]
    fn a_narrow_terminal_drops_whole_columns_never_half_a_value() {
        let mins = [0, 60, 100];
        assert_eq!(columns_that_fit(&mins, 120), vec![0, 1, 2]);
        assert_eq!(columns_that_fit(&mins, 80), vec![0, 1]);
        assert_eq!(columns_that_fit(&mins, 40), vec![0]);
    }
}
