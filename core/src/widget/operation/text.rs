//! Operate on widgets that have text.
use crate::Point;
use crate::Vector;
use crate::text::{Fragment, Target};
use crate::widget::operation::{Operation, Outcome};

/// The internal state of a widget that has text.
pub trait Text {
    /// Returns the text of the widget.
    fn text(&self) -> Fragment<'_>;

    /// Selects text between the given positions in layout coordinates.
    ///
    /// `start` is the position where the user started selecting from,
    /// while `end` is the final position of the selection.
    fn select(&mut self, start: Point, end: Point, target: Target);

    /// Selects all the text of the widget.
    fn select_all(&mut self);

    /// Deselects any selected text;
    fn deselect(&mut self);

    /// Returns the selected text, if any.
    ///
    // TODO: Make immutable
    fn copy(&mut self) -> Option<String>;
}

impl Text for String {
    fn text(&self) -> Fragment<'_> {
        self.as_str().into()
    }

    fn select(&mut self, _start: Point, _end: Point, _target: Target) {}
    fn select_all(&mut self) {}
    fn deselect(&mut self) {}

    fn copy(&mut self) -> Option<String> {
        None
    }
}

/// Selects text between the given positions in layout coordinates.
///
/// `start` is the position where the user started selecting from,
/// while `end` is the final position of the selection.
pub fn select(start: Point, end: Point, target: Target) -> impl Operation {
    struct Select {
        start: Point,
        end: Point,
        target: Target,
        translation: Vector,
    }

    impl Operation for Select {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            let translation = self.translation;
            operate(self);
            self.translation = translation;
        }

        fn scrollable(
            &mut self,
            _id: Option<&crate::widget::Id>,
            _bounds: crate::Rectangle,
            _content: crate::Size,
            translation: crate::Vector,
            _state: &mut dyn super::Scrollable,
        ) {
            self.translation += translation;
        }

        fn text(
            &mut self,
            _id: Option<&crate::widget::Id>,
            _bounds: crate::Rectangle,
            state: &mut dyn Text,
        ) {
            state.select(
                self.start + self.translation,
                self.end + self.translation,
                self.target,
            );
        }
    }

    Select {
        start,
        end,
        target,
        translation: Vector::ZERO,
    }
}

/// Selects all selectable text.
pub fn select_all() -> impl Operation {
    struct SelectAll;

    impl Operation for SelectAll {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn text(
            &mut self,
            _id: Option<&crate::widget::Id>,
            _bounds: crate::Rectangle,
            state: &mut dyn Text,
        ) {
            state.select_all();
        }
    }

    SelectAll
}

/// Deselects any selected text.
pub fn deselect() -> impl Operation {
    struct Deselect;

    impl Operation for Deselect {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn text(
            &mut self,
            _id: Option<&crate::widget::Id>,
            _bounds: crate::Rectangle,
            state: &mut dyn Text,
        ) {
            state.deselect();
        }
    }

    Deselect
}

/// Collects all the selected text.
pub fn copy() -> impl Operation<String> {
    struct Copy {
        text: String,
    }

    impl Operation<String> for Copy {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<String>)) {
            operate(self);
        }

        fn text(
            &mut self,
            _id: Option<&crate::widget::Id>,
            _bounds: crate::Rectangle,
            state: &mut dyn Text,
        ) {
            let Some(text) = state.copy() else {
                return;
            };

            if text.is_empty() {
                return;
            }

            if !self.text.is_empty() {
                self.text.push('\n');
            }

            self.text.push_str(&text);
        }

        fn finish(&self) -> Outcome<String> {
            if self.text.is_empty() {
                Outcome::None
            } else {
                Outcome::Some(self.text.clone())
            }
        }
    }

    Copy {
        text: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rectangle;

    struct Selected(Option<&'static str>);

    impl Text for Selected {
        fn text(&self) -> Fragment<'_> {
            "".into()
        }

        fn select(&mut self, _start: Point, _end: Point, _target: Target) {}
        fn select_all(&mut self) {}
        fn deselect(&mut self) {}

        fn copy(&mut self) -> Option<String> {
            self.0.map(str::to_owned)
        }
    }

    fn copy_from(states: &mut [Selected]) -> Option<String> {
        let mut copy = copy();

        for state in states {
            copy.text(None, Rectangle::default(), state);
        }

        match copy.finish() {
            Outcome::Some(text) => Some(text),
            Outcome::None => None,
            Outcome::Chain(_) => panic!("copy does not chain"),
        }
    }

    #[test]
    fn copy_without_a_selection_produces_nothing() {
        assert_eq!(copy_from(&mut []), None);
        assert_eq!(copy_from(&mut [Selected(None)]), None);
        assert_eq!(copy_from(&mut [Selected(Some("")), Selected(None)]), None);
    }

    #[test]
    fn copy_joins_every_selection() {
        assert_eq!(
            copy_from(&mut [Selected(Some("a")), Selected(None), Selected(Some("b"))]),
            Some("a\nb".to_owned())
        );
    }
}
