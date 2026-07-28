//! The list filter shared by every CLI listing and every TUI table.

use regex::RegexBuilder;

/// A case-insensitive substring match that ALSO accepts a regex.
///
/// Invalid regex input is never an error while typing; it simply falls back to
/// substring matching until the pattern becomes valid. Anything else would make
/// the filter flash an error on the way to every `(` the user meant to type.
pub struct FilterMatcher<'a> {
    raw: &'a str,
    lower: String,
    regex: Option<regex::Regex>,
}

impl<'a> FilterMatcher<'a> {
    pub fn new(raw: &'a str) -> Self {
        Self {
            raw,
            lower: raw.to_ascii_lowercase(),
            regex: if raw.is_empty() {
                None
            } else {
                RegexBuilder::new(raw).case_insensitive(true).build().ok()
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    pub fn matches(&self, cell: &str) -> bool {
        self.is_empty()
            || cell.to_ascii_lowercase().contains(&self.lower)
            || self.regex.as_ref().is_some_and(|re| re.is_match(cell))
    }

    pub fn matches_any<'b, I>(&self, cells: I) -> bool
    where
        I: IntoIterator<Item = &'b str>,
    {
        self.is_empty() || cells.into_iter().any(|cell| self.matches(cell))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_filter_keeps_everything() {
        assert!(FilterMatcher::new("").matches("anything"));
    }

    #[test]
    fn matching_ignores_case() {
        assert!(FilterMatcher::new("WEB").matches("my-web-app"));
    }

    #[test]
    fn a_regex_matches_too() {
        assert!(FilterMatcher::new("^api-.*-prod$").matches("api-billing-prod"));
        assert!(!FilterMatcher::new("^api-.*-prod$").matches("api-billing-staging"));
    }

    #[test]
    fn a_half_typed_regex_is_not_an_error() {
        // "(web" is invalid; it must still match as a substring rather than
        // filtering the whole table away while the user is mid-keystroke.
        let m = FilterMatcher::new("(web");
        assert!(m.matches("weird(web)"));
        assert!(!m.matches("database"));
    }

    #[test]
    fn a_row_matches_when_any_of_its_cells_does() {
        let m = FilterMatcher::new("running");
        assert!(m.matches_any(["web", "running", "2"]));
        assert!(!m.matches_any(["web", "stopped", "2"]));
    }
}
