//! Platform specific settings for macOS.

/// The platform specific window settings of an application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformSpecific {
    /// Hides the window title.
    pub title_hidden: bool,
    /// Makes the titlebar transparent and allows the content to appear behind it.
    pub titlebar_transparent: bool,
    /// Makes the window content appear behind the titlebar.
    pub fullsize_content_view: bool,
    /// Whether a mouse down on the window's content may move the window.
    ///
    /// With a transparent titlebar over full-size content, AppKit lets a press on the
    /// content under the titlebar drag the window before the application sees it, so a
    /// widget drawn there (a tab strip) cannot be dragged on its own. Set it to `false`
    /// to keep every press for the application, which then moves the window itself
    /// (`window::drag`) from the areas it wants draggable.
    ///
    /// Defaults to `true` (AppKit's own behavior).
    pub mouse_down_can_move_window: bool,
    /// Gives the window a unified titlebar with the compact toolbar style: a
    /// titlebar taller than the plain one, with the window controls centred in it.
    /// In native fullscreen it hides with the menu bar.
    pub unified_titlebar_compact: bool,
}

impl Default for PlatformSpecific {
    fn default() -> Self {
        Self {
            title_hidden: false,
            titlebar_transparent: false,
            fullsize_content_view: false,
            mouse_down_can_move_window: true,
            unified_titlebar_compact: false,
        }
    }
}
