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
            /// What `arboard` would have used here: a windowless X11
            /// connection. Kept for READING only, and only when the
            /// data device answers with nothing.
            ///
            /// A compositor without data-control is usually bridging
            /// some other clipboard into Wayland, and the bridge can be
            /// one-way. WSLg is exactly that: its data device hands back
            /// an empty string for the host clipboard while the same
            /// content reads correctly over X11. Writing still goes to
            /// the data device, which is the direction that needed this
            /// fallback removed in the first place.
            x11_read: Option<Arc<Mutex<arboard::Clipboard>>>,
            // The raw `wl_display` handed to `smithay_clipboard` must outlive it.
            _display_handle: winit::event_loop::OwnedDisplayHandle,
        },
        Unavailable,
    }

    /// Read text from the windowless X11 connection kept beside the
    /// Wayland data device, if there is one.
    ///
    /// The error type matches the data device's (`std::io`), because this
    /// is a second attempt at the same question and the caller only cares
    /// whether an answer arrived. Having no X11 clipboard and failing to
    /// read one are the same "no", so both come back as the same `Err`.
    #[cfg(all(target_os = "linux", feature = "wayland"))]
    fn read_x11_text(
        clipboard: Option<&Arc<Mutex<arboard::Clipboard>>>,
        clipboard_kind: ClipboardKind,
    ) -> std::io::Result<String> {
        use std::io::{Error as IoError, ErrorKind};

        let missing = || {
            IoError::new(
                ErrorKind::NotFound,
                "no X11 clipboard to fall back to",
            )
        };

        let Some(clipboard) = clipboard else {
            return Err(missing());
        };
        let Ok(mut clipboard) = clipboard.lock() else {
            return Err(missing());
        };
        get_clipboard(&mut clipboard, clipboard_kind)
            .text()
            .map_err(|error| {
                log::debug!("x11 clipboard read fallback failed: {error}");

                missing()
            })
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
                            x11_read: arboard::Clipboard::new()
                                .ok()
                                .map(|clipboard| Arc::new(Mutex::new(clipboard))),
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
                State::WaylandDataDevice {
                    clipboard, x11_read, ..
                } => {
                    let clipboard = clipboard.clone();
                    let x11_read = x11_read.clone();

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
                                // EMPTY counts as no answer, not as an
                                // empty clipboard. A compositor without
                                // data-control is usually bridging another
                                // clipboard in, and the bridge can be
                                // one-way: WSLg answers the data device
                                // with "" for content that reads fine over
                                // X11. Asking the other side costs nothing
                                // when the clipboard really is empty,
                                // because then it answers empty too.
                                let contents = match contents {
                                    Ok(text) if !text.is_empty() => Ok(text),
                                    other => {
                                        if let Err(error) = &other {
                                            log::debug!(
                                                "wayland clipboard read failed: \
                                                 {error}"
                                            );
                                        }
                                        read_x11_text(
                                            x11_read.as_ref(),
                                            clipboard_kind,
                                        )
                                        .or(other)
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
