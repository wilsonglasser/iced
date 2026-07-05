//! Change internal widget state.
use crate::core::widget::Id;
use crate::core::widget::operation;
use crate::task;
use crate::{Action, Task};

pub use crate::core::widget::operation::scrollable::{AbsoluteOffset, RelativeOffset};

/// Snaps the scrollable with the given [`Id`] to the provided [`RelativeOffset`].
pub fn snap_to<T>(id: impl Into<Id>, offset: impl Into<RelativeOffset<Option<f32>>>) -> Task<T> {
    task::effect(Action::widget(operation::scrollable::snap_to(
        id.into(),
        offset.into(),
    )))
}

/// Snaps the scrollable with the given [`Id`] to the [`RelativeOffset::END`].
pub fn snap_to_end<T>(id: impl Into<Id>) -> Task<T> {
    task::effect(Action::widget(operation::scrollable::snap_to(
        id.into(),
        RelativeOffset::END.into(),
    )))
}

/// Scrolls the scrollable with the given [`Id`] to the provided [`AbsoluteOffset`].
pub fn scroll_to<T>(id: impl Into<Id>, offset: impl Into<AbsoluteOffset<Option<f32>>>) -> Task<T> {
    task::effect(Action::widget(operation::scrollable::scroll_to(
        id.into(),
        offset.into(),
    )))
}

/// Scrolls the scrollable with the given [`Id`] by the provided [`AbsoluteOffset`].
pub fn scroll_by<T>(id: impl Into<Id>, offset: AbsoluteOffset) -> Task<T> {
    task::effect(Action::widget(operation::scrollable::scroll_by(
        id.into(),
        offset,
    )))
}

/// Focuses the previous focusable widget.
pub fn focus_previous<T>() -> Task<T> {
    task::effect(Action::widget(operation::focusable::focus_previous()))
}

/// Focuses the next focusable widget.
pub fn focus_next<T>() -> Task<T> {
    task::effect(Action::widget(operation::focusable::focus_next()))
}

/// Returns whether the widget with the given [`Id`] is focused or not.
pub fn is_focused(id: impl Into<Id>) -> Task<bool> {
    task::widget(operation::focusable::is_focused(id.into()))
}

/// Finds the [`Id`] of the currently focused widget, if any.
///
/// Unlike [`operation::focusable::find_focused`], which produces no value
/// when nothing is focused, this always yields (`None` in that case), so a
/// caller can react either way in a single message.
pub fn find_focused() -> Task<Option<Id>> {
    use crate::core::Rectangle;
    use crate::core::widget::operation::{Focusable, Operation, Outcome};

    struct FindFocused {
        focused: Option<Id>,
    }

    impl Operation<Option<Id>> for FindFocused {
        fn traverse(
            &mut self,
            operate: &mut dyn FnMut(&mut dyn Operation<Option<Id>>),
        ) {
            operate(self);
        }

        fn focusable(
            &mut self,
            id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn Focusable,
        ) {
            if state.is_focused() && id.is_some() {
                self.focused = id.cloned();
            }
        }

        fn finish(&self) -> Outcome<Option<Id>> {
            Outcome::Some(self.focused.clone())
        }
    }

    task::widget(FindFocused { focused: None })
}

/// Focuses the widget with the given [`Id`].
pub fn focus<T>(id: impl Into<Id>) -> Task<T> {
    task::effect(Action::widget(operation::focusable::focus(id.into())))
}

/// Moves the cursor of the widget with the given [`Id`] to the end.
pub fn move_cursor_to_end<T>(id: impl Into<Id>) -> Task<T> {
    task::effect(Action::widget(operation::text_input::move_cursor_to_end(
        id.into(),
    )))
}

/// Moves the cursor of the widget with the given [`Id`] to the front.
pub fn move_cursor_to_front<T>(id: impl Into<Id>) -> Task<T> {
    task::effect(Action::widget(operation::text_input::move_cursor_to_front(
        id.into(),
    )))
}

/// Moves the cursor of the widget with the given [`Id`] to the provided position.
pub fn move_cursor_to<T>(id: impl Into<Id>, position: usize) -> Task<T> {
    task::effect(Action::widget(operation::text_input::move_cursor_to(
        id.into(),
        position,
    )))
}

/// Selects all the content of the widget with the given [`Id`].
pub fn select_all<T>(id: impl Into<Id>) -> Task<T> {
    task::effect(Action::widget(operation::text_input::select_all(id.into())))
}

/// Selects the given content range of the widget with the given [`Id`].
pub fn select_range<T>(id: impl Into<Id>, start: usize, end: usize) -> Task<T> {
    task::effect(Action::widget(operation::text_input::select_range(
        id.into(),
        start,
        end,
    )))
}
