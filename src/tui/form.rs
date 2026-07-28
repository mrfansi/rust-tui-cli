//! A modal form: fields, focus, conditional visibility, validation.
//!
//! One `Form` type for every dialog in the tool. A new dialog is a `FormKind`
//! variant plus a function returning `Vec<Field>` — not a new widget, not new key
//! handling, and not a second place where Tab-moves-focus has to be implemented.

#[derive(Clone)]
pub(super) enum FieldKind {
    Text,
    /// Rendered as dots. Still typed into normally.
    Secret,
    /// A choice from real data — cycled with Space / ← / →, so the options can
    /// come from the API instead of being typed from memory.
    Choice(Vec<String>),
}

impl FieldKind {
    pub(super) fn is_typed(&self) -> bool {
        matches!(self, FieldKind::Text | FieldKind::Secret)
    }
}

pub(super) struct Field {
    pub(super) label: &'static str,
    pub(super) value: String,
    pub(super) kind: FieldKind,
    pub(super) required: bool,
    /// Show conditions, AND-combined: (label of the deciding field, accepted
    /// values separated by commas). Empty = always shown.
    ///
    /// Swapping the fields below a choice is what keeps one form from becoming
    /// three, and one form is what keeps the submit path from becoming three.
    pub(super) only_for: Vec<(&'static str, &'static str)>,
}

impl Field {
    pub(super) fn text(label: &'static str, value: &str) -> Self {
        Self {
            label,
            value: value.into(),
            kind: FieldKind::Text,
            required: false,
            only_for: Vec::new(),
        }
    }

    pub(super) fn secret(label: &'static str) -> Self {
        Self {
            kind: FieldKind::Secret,
            ..Self::text(label, "")
        }
    }

    pub(super) fn choice(label: &'static str, options: Vec<String>) -> Self {
        let first = options.first().cloned().unwrap_or_default();
        Self {
            kind: FieldKind::Choice(options),
            ..Self::text(label, &first)
        }
    }

    pub(super) fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Show this field only while `switch` holds one of `values` (comma
    /// separated). Callable more than once; the conditions are AND-combined.
    pub(super) fn when(mut self, switch: &'static str, values: &'static str) -> Self {
        self.only_for.push((switch, values));
        self
    }
}

/// Which dialog this is — read on submit to decide what to send.
#[derive(PartialEq, Clone, Copy)]
pub(super) enum FormKind {
    NewItem,
    AddProfile,
}

pub(super) struct Form {
    pub(super) kind: FormKind,
    pub(super) title: String,
    pub(super) fields: Vec<Field>,
    /// Index into `fields`, not into the visible subset: a field that scrolls out
    /// of view when a choice changes must not silently move focus to its neighbour.
    pub(super) focus: usize,
    /// Why the last submit was refused. Drawn on the form's own border — the
    /// status line fades after a few seconds, which would erase the explanation
    /// while the user is still looking for the field it names.
    pub(super) error: Option<String>,
}

impl Form {
    pub(super) fn new(kind: FormKind, title: impl Into<String>, fields: Vec<Field>) -> Self {
        Self {
            kind,
            title: title.into(),
            fields,
            focus: 0,
            error: None,
        }
    }

    /// Indices of the fields shown right now.
    pub(super) fn visible(&self) -> Vec<usize> {
        self.fields
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.only_for.iter().all(|(switch, values)| {
                    let cur = self.value(switch);
                    values.split(',').any(|v| v == cur)
                })
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub(super) fn value(&self, label: &str) -> String {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map(|f| f.value.clone())
            .unwrap_or_default()
    }

    /// Move focus by `delta` through the VISIBLE fields, stopping at both ends.
    pub(super) fn move_focus(&mut self, delta: isize) {
        let visible = self.visible();
        if visible.is_empty() {
            return;
        }
        let at = visible.iter().position(|&i| i == self.focus).unwrap_or(0) as isize;
        let next = (at + delta).clamp(0, visible.len() as isize - 1) as usize;
        self.focus = visible[next];
    }

    fn focused(&mut self) -> Option<&mut Field> {
        self.fields.get_mut(self.focus)
    }

    pub(super) fn type_char(&mut self, c: char) {
        if let Some(f) = self.focused() {
            if f.kind.is_typed() {
                f.value.push(c);
            }
        }
    }

    pub(super) fn backspace(&mut self) {
        if let Some(f) = self.focused() {
            if f.kind.is_typed() {
                f.value.pop();
            }
        }
    }

    /// Step a Choice field forward (`1`) or back (`-1`), wrapping.
    ///
    /// Wrapping is right here and wrong for a table: a three-option cycle that
    /// stops at the end makes the user step backwards to reach an option they
    /// just passed.
    pub(super) fn cycle(&mut self, delta: isize) {
        let Some(f) = self.focused() else { return };
        let FieldKind::Choice(options) = &f.kind else {
            return;
        };
        if options.is_empty() {
            return;
        }
        let at = options.iter().position(|o| *o == f.value).unwrap_or(0) as isize;
        let n = options.len() as isize;
        f.value = options[(at + delta).rem_euclid(n) as usize].clone();
    }

    /// The first visible required field left empty, if any.
    ///
    /// Only VISIBLE fields count: a required field hidden by a choice the user
    /// didn't make is not something they can fill in, and blocking on it produces
    /// a form that cannot be submitted and won't say why.
    pub(super) fn validate(&self) -> Result<(), String> {
        for i in self.visible() {
            let f = &self.fields[i];
            if f.required && f.value.trim().is_empty() {
                return Err(format!("{} is required", f.label));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> Form {
        Form::new(
            FormKind::NewItem,
            "New item",
            vec![
                Field::text("Name", "").required(),
                Field::choice("Kind", vec!["app".into(), "db".into()]),
                Field::text("Image", "").required().when("Kind", "db"),
            ],
        )
    }

    #[test]
    fn a_field_is_hidden_until_its_switch_selects_it() {
        let mut f = form();
        assert_eq!(f.visible(), vec![0, 1], "Image belongs to Kind=db");
        f.focus = 1;
        f.cycle(1);
        assert_eq!(f.value("Kind"), "db");
        assert_eq!(f.visible(), vec![0, 1, 2]);
    }

    #[test]
    fn a_hidden_required_field_does_not_block_the_form() {
        // Otherwise the form refuses to submit over a field the user cannot see.
        let mut f = form();
        f.fields[0].value = "web".into();
        assert!(f.validate().is_ok());
        f.focus = 1;
        f.cycle(1);
        assert_eq!(f.validate(), Err("Image is required".into()));
    }

    #[test]
    fn focus_skips_hidden_fields_and_stops_at_the_ends() {
        let mut f = form();
        f.move_focus(-1);
        assert_eq!(f.focus, 0, "already at the top");
        f.move_focus(1);
        f.move_focus(1);
        assert_eq!(f.focus, 1, "Image is hidden, so Kind is the last one");
    }

    #[test]
    fn a_choice_cycles_in_both_directions() {
        let mut f = form();
        f.focus = 1;
        f.cycle(-1);
        assert_eq!(
            f.value("Kind"),
            "db",
            "backwards from the first wraps to the last"
        );
        f.cycle(-1);
        assert_eq!(f.value("Kind"), "app");
    }

    #[test]
    fn typing_never_lands_in_a_choice_field() {
        let mut f = form();
        f.focus = 1;
        f.type_char('x');
        assert_eq!(f.value("Kind"), "app", "a choice is picked, not typed");
    }
}
