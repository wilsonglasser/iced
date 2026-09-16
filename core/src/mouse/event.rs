use crate::Point;

use super::Button;

/// A mouse event.
///
/// _**Note:** This type is largely incomplete! If you need to track
/// additional events, feel free to [open an issue] and share your use case!_
///
/// [open an issue]: https://github.com/iced-rs/iced/issues
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// The mouse cursor entered the window.
    CursorEntered,

    /// The mouse cursor left the window.
    CursorLeft,

    /// The mouse cursor was moved
    CursorMoved {
        /// The new position of the mouse cursor
        position: Point,
    },

    /// A mouse button was pressed.
    ButtonPressed(Button),

    /// A mouse button was released.
    ButtonReleased(Button),

    /// The mouse wheel was scrolled.
    WheelScrolled {
        /// The scroll movement.
        delta: ScrollDelta,
    },

    /// A two-finger pinch on a touchpad, the magnification gesture.
    ///
    /// Only some platforms deliver it as a gesture (macOS and Wayland
    /// do; Windows turns a precision-touchpad pinch into a
    /// Ctrl + [`Event::WheelScrolled`] before the window sees it).
    Pinched {
        /// The change in magnification since the previous event of the
        /// same gesture: positive when the fingers spread (zoom in),
        /// negative when they close (zoom out). `0.0` on the phase
        /// events that open and close a gesture.
        delta: f32,
        /// Where in the gesture this event sits.
        phase: GesturePhase,
    },
}

/// The lifecycle of a touchpad gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GesturePhase {
    /// The fingers landed; the gesture begins.
    Started,
    /// The fingers moved; the event carries a delta.
    Moved,
    /// The fingers lifted; the gesture is complete.
    Ended,
    /// The system abandoned the gesture (another finger landed, the
    /// window lost the pointer, ...).
    Cancelled,
}

/// A scroll movement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollDelta {
    /// A line-based scroll movement
    Lines {
        /// The number of horizontal lines scrolled
        x: f32,

        /// The number of vertical lines scrolled
        y: f32,
    },
    /// A pixel-based scroll movement
    Pixels {
        /// The number of horizontal pixels scrolled
        x: f32,
        /// The number of vertical pixels scrolled
        y: f32,
    },
}
