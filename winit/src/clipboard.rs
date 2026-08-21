//! Access the clipboard.
use crate::core::clipboard::{ClipboardKind, Content, Error, Kind};

pub use platform::*;

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod platform {
    #[cfg(target_os = "linux")]
    use arboard::{GetExtLinux, SetExtLinux};

    use super::*;

    use std::sync::{Arc, Mutex};
    use std::thread;

    /// A buffer for short-term storage and transfer within and between
    /// applications.
    pub struct Clipboard {
        state: State,
    }

    enum State {
        Connected {
            clipboard: Arc<Mutex<arboard::Clipboard>>,
        },
        #[cfg(all(target_os = "linux", feature = "wayland"))]
        WaylandDataDevice {
            clipboard: Arc<Mutex<smithay_clipboard::Clipboard>>,
            // The raw `wl_display` handed to `smithay_clipboard` must outlive it.
            _display_handle: winit::event_loop::OwnedDisplayHandle,
        },
        Unavailable,
    }

    impl Clipboard {
        /// Creates a new [`Clipboard`] for the given window.
        pub fn new() -> Self {
            let clipboard = arboard::Clipboard::new();

            let state = match clipboard {
                Ok(clipboard) => State::Connected {
                    clipboard: Arc::new(Mutex::new(clipboard)),
                },
                Err(_) => State::Unavailable,
            };

            Clipboard { state }
        }

        /// Creates a new [`Clipboard`] for the display behind the given handle.
        ///
        /// On Wayland sessions whose compositor does not offer the data-control
        /// protocol (ChromeOS's sommelier, Weston, Mutter < 47), `arboard` would
        /// silently fall back to a windowless X11 connection whose selection never
        /// reaches the compositor's clipboard; this constructor detects that case
        /// and uses a data-device clipboard tied to our own seat instead.
        pub fn connect(
            display_handle: &winit::event_loop::OwnedDisplayHandle,
        ) -> Self {
            #[cfg(all(target_os = "linux", feature = "wayland"))]
            {
                use raw_window_handle::{HasDisplayHandle as _, RawDisplayHandle};

                if let Ok(handle) = display_handle.display_handle()
                    && let RawDisplayHandle::Wayland(wayland) = handle.as_raw()
                    && !data_control_available()
                {
                    // SAFETY: the display pointer stays valid for the
                    // lifetime of the `OwnedDisplayHandle` clone stored
                    // alongside the clipboard.
                    #[allow(unsafe_code)]
                    let clipboard = unsafe {
                        smithay_clipboard::Clipboard::new(
                            wayland.display.as_ptr(),
                        )
                    };

                    return Clipboard {
                        state: State::WaylandDataDevice {
                            clipboard: Arc::new(Mutex::new(clipboard)),
                            _display_handle: display_handle.clone(),
                        },
                    };
                }
            }

            #[cfg(not(all(target_os = "linux", feature = "wayland")))]
            let _ = display_handle;

            Self::new()
        }

        /// Reads the current content of the [`Clipboard`] as text.
        pub fn read(
            &self,
            clipboard_kind: ClipboardKind,
            kind: Kind,
            callback: impl FnOnce(Result<Content, Error>) + Send + 'static,
        ) {
            let clipboard = match &self.state {
                State::Connected { clipboard } => clipboard.clone(),
                #[cfg(all(target_os = "linux", feature = "wayland"))]
                State::WaylandDataDevice { clipboard, .. } => {
                    let clipboard = clipboard.clone();

                    let _ = thread::spawn(move || {
                        let Ok(clipboard) = clipboard.lock() else {
                            callback(Err(Error::ClipboardUnavailable));
                            return;
                        };

                        let result = match kind {
                            Kind::Text => {
                                let contents = match clipboard_kind {
                                    ClipboardKind::Standard => clipboard.load(),
                                    ClipboardKind::Primary => {
                                        clipboard.load_primary()
                                    }
                                };

                                contents.map(Content::Text).map_err(|error| {
                                    log::debug!(
                                        "wayland clipboard read failed: {error}"
                                    );

                                    Error::ContentNotAvailable
                                })
                            }
                            kind => {
                                log::warn!(
                                    "unsupported clipboard kind on the wayland \
                                     data-device fallback: {kind:?}"
                                );

                                Err(Error::ContentNotAvailable)
                            }
                        };

                        callback(result);
                    });

                    return;
                }
                State::Unavailable => {
                    callback(Err(Error::ClipboardUnavailable));
                    return;
                }
            };

            let _ = thread::spawn(move || {
                let Ok(mut clipboard) = clipboard.lock() else {
                    callback(Err(Error::ClipboardUnavailable));
                    return;
                };

                let get = get_clipboard(&mut clipboard, clipboard_kind);

                let result = match kind {
                    Kind::Text => get.text().map(Content::Text),
                    Kind::Html => get.html().map(Content::Html),
                    #[cfg(feature = "image")]
                    Kind::Image => get.image().map(|image| {
                        let rgba = crate::core::Bytes::from_owner(image.bytes);
                        let size = crate::core::Size {
                            width: image.width as u32,
                            height: image.height as u32,
                        };

                        Content::Image(crate::core::clipboard::Image { rgba, size })
                    }),
                    Kind::Files => get.file_list().map(Content::Files),
                    kind => {
                        log::warn!("unsupported clipboard kind: {kind:?}");

                        Err(arboard::Error::ContentNotAvailable)
                    }
                }
                .map_err(to_error);

                callback(result);
            });
        }

        /// Writes the given text contents to the [`Clipboard`].
        pub fn write(
            &mut self,
            clipboard_kind: ClipboardKind,
            content: Content,
            callback: impl FnOnce(Result<(), Error>) + Send + 'static,
        ) {
            let clipboard = match &self.state {
                State::Connected { clipboard } => clipboard.clone(),
                #[cfg(all(target_os = "linux", feature = "wayland"))]
                State::WaylandDataDevice { clipboard, .. } => {
                    let clipboard = clipboard.clone();

                    let _ = thread::spawn(move || {
                        let Ok(clipboard) = clipboard.lock() else {
                            callback(Err(Error::ClipboardUnavailable));
                            return;
                        };

                        let result = match content {
                            Content::Text(text) => {
                                match clipboard_kind {
                                    ClipboardKind::Standard => {
                                        clipboard.store(text);
                                    }
                                    ClipboardKind::Primary => {
                                        clipboard.store_primary(text);
                                    }
                                }

                                Ok(())
                            }
                            content => {
                                log::warn!(
                                    "unsupported clipboard content on the \
                                     wayland data-device fallback: {content:?}"
                                );

                                Err(Error::ClipboardUnavailable)
                            }
                        };

                        callback(result);
                    });

                    return;
                }
                State::Unavailable => {
                    callback(Err(Error::ClipboardUnavailable));
                    return;
                }
            };

            let _ = thread::spawn(move || {
                let Ok(mut clipboard) = clipboard.lock() else {
                    callback(Err(Error::ClipboardUnavailable));
                    return;
                };

                let set = set_clipboard(&mut clipboard, clipboard_kind);

                let result = match content {
                    Content::Text(text) => set.text(text),
                    Content::Html(html) => set.html(html, None),
                    #[cfg(feature = "image")]
                    Content::Image(image) => set.image(arboard::ImageData {
                        bytes: image.rgba.as_ref().into(),
                        width: image.size.width as usize,
                        height: image.size.height as usize,
                    }),
                    Content::Files(files) => set.file_list(&files),
                    content => {
                        log::warn!("unsupported clipboard content: {content:?}");

                        Err(arboard::Error::ClipboardNotSupported)
                    }
                }
                .map_err(to_error);

                callback(result);
            });
        }
    }

    /// Whether the Wayland compositor offers a data-control protocol
    /// (`zwlr_data_control_manager_v1` or `ext_data_control_manager_v1`),
    /// which is the only protocol `arboard` speaks on Wayland.
    #[cfg(all(target_os = "linux", feature = "wayland"))]
    fn data_control_available() -> bool {
        // `Ok(_)` means the data-control manager itself was found, regardless
        // of whether the primary selection is supported on top of it.
        wl_clipboard_rs::utils::is_primary_selection_supported().is_ok()
    }

    #[cfg(target_os = "linux")]
    fn get_clipboard(clipboard: &mut arboard::Clipboard, kind: ClipboardKind) -> arboard::Get<'_> {
        clipboard.get().clipboard(match kind {
            ClipboardKind::Standard => arboard::LinuxClipboardKind::Clipboard,
            ClipboardKind::Primary => arboard::LinuxClipboardKind::Primary,
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn get_clipboard(clipboard: &mut arboard::Clipboard, kind: ClipboardKind) -> arboard::Get<'_> {
        let _ = kind;
        clipboard.get()
    }

    #[cfg(target_os = "linux")]
    fn set_clipboard(clipboard: &mut arboard::Clipboard, kind: ClipboardKind) -> arboard::Set<'_> {
        clipboard.set().clipboard(match kind {
            ClipboardKind::Standard => arboard::LinuxClipboardKind::Clipboard,
            ClipboardKind::Primary => arboard::LinuxClipboardKind::Primary,
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn set_clipboard(clipboard: &mut arboard::Clipboard, kind: ClipboardKind) -> arboard::Set<'_> {
        let _ = kind;
        clipboard.set()
    }

    fn to_error(error: arboard::Error) -> Error {
        match error {
            arboard::Error::ContentNotAvailable => Error::ContentNotAvailable,
            arboard::Error::ClipboardNotSupported => Error::ClipboardUnavailable,
            arboard::Error::ClipboardOccupied => Error::ClipboardOccupied,
            arboard::Error::ConversionFailure => Error::ConversionFailure,
            arboard::Error::Unknown { description } => Error::Unknown {
                description: Arc::new(description),
            },
            error => Error::Unknown {
                description: Arc::new(error.to_string()),
            },
        }
    }
}

// TODO: Wasm support
#[cfg(target_arch = "wasm32")]
mod platform {
    use super::*;

    /// A buffer for short-term storage and transfer within and between
    /// applications.
    pub struct Clipboard;

    impl Clipboard {
        /// Creates a new [`Clipboard`] for the given window.
        pub fn new() -> Self {
            Self
        }

        /// Creates a new [`Clipboard`] for the display behind the given handle.
        pub fn connect(
            _display_handle: &winit::event_loop::OwnedDisplayHandle,
        ) -> Self {
            Self::new()
        }

        /// Reads the current content of the [`Clipboard`] as text.
        pub fn read(
            &self,
            _clipboard_kind: ClipboardKind,
            _kind: Kind,
            callback: impl FnOnce(Result<Content, Error>),
        ) {
            callback(Err(Error::ClipboardUnavailable));
        }

        /// Writes the given text contents to the [`Clipboard`].
        pub fn write(
            &mut self,
            _clipboard_kind: ClipboardKind,
            _content: Content,
            callback: impl FnOnce(Result<(), Error>),
        ) {
            callback(Err(Error::ClipboardUnavailable));
        }
    }
}
