use crate::time::Instant;
use crate::{Point, Size};

use std::path::PathBuf;

/// A window-related event.
#[derive(PartialEq, Clone, Debug)]
pub enum Event {
    /// A window was opened.
    Opened {
        /// The position of the opened window. This is relative to the top-left corner of the desktop
        /// the window is on, including virtual desktops. Refers to window's "outer" position,
        /// or the window area, in logical pixels.
        ///
        /// **Note**: Not available in Wayland.
        position: Option<Point>,
        /// The size of the created window. This is its "inner" size, or the size of the
        /// client area, in logical pixels.
        size: Size,
        /// The scale factor of the created window.
        scale_factor: f32,
    },

    /// A window was closed.
    Closed,

    /// A window was moved.
    Moved(Point),

    /// A window was resized.
    Resized(Size),

    /// A window changed its scale factor.
    Rescaled(f32),

    /// A window redraw was requested.
    ///
    /// The [`Instant`] contains the current time.
    RedrawRequested(Instant),

    /// The user has requested for the window to close.
    CloseRequested,

    /// A window was focused.
    Focused,

    /// A window was unfocused.
    Unfocused,

    /// A file is being hovered over the window.
    ///
    /// When the user hovers multiple files at once, this event will be emitted
    /// for each file separately.
    ///
    /// ## Platform-specific
    ///
    /// - **Wayland:** Not implemented.
    FileHovered(PathBuf),

    /// A file has been dropped into the window.
    ///
    /// When the user drops multiple files at once, this event will be emitted
    /// for each file separately.
    ///
    /// ## Platform-specific
    ///
    /// - **Wayland:** Not implemented.
    FileDropped(PathBuf),

    /// A file was hovered, but has exited the window.
    ///
    /// There will be a single `FilesHoveredLeft` event triggered even if
    /// multiple files were hovered.
    ///
    /// ## Platform-specific
    ///
    /// - **Wayland:** Not implemented.
    FilesHoveredLeft,

    /// A window that is being carried by a drag (see `window::drag_toplevel`
    /// in the runtime) is over this window, at the given position.
    ///
    /// The carried window takes no part in the drag: it is the windows
    /// under it that are told where the cursor is, so that one of them can
    /// offer itself as the place to drop.
    ///
    /// ## Platform-specific
    ///
    /// - **Wayland only.**
    ToplevelDragMoved {
        /// The position of the cursor in this window, in logical pixels.
        position: Point,
    },

    /// The carried window left this window without being dropped on it.
    ToplevelDragLeft,

    /// The carried window was dropped on this window.
    ToplevelDragDropped,

    /// The drag that was carrying a window ended. Delivered to the window
    /// the drag started from, whether or not it was dropped on a window.
    ToplevelDragEnded,
}
