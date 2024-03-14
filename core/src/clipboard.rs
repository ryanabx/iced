//! Access the clipboard.

use window_clipboard::mime::{self, AllowedMimeTypes, ClipboardStoreData};

/// A buffer for short-term storage and transfer within and between
/// applications.
pub trait Clipboard {
    /// Reads the current content of the [`Clipboard`] as text.
    fn read(&self, kind: Kind) -> Option<String>;

    /// Writes the given text contents to the [`Clipboard`].
    fn write(&mut self, kind: Kind, contents: String);

    /// Consider using [`read_data`] instead
    /// Reads the current content of the [`Clipboard`] as text.
    fn read_data(&self, kind: Kind, _mimes: Vec<String>) -> Option<(Vec<u8>, String)> {
        None
    }

    /// Writes the given contents to the [`Clipboard`].
    fn write_data(
        &mut self,
        kind: Kind,
        _contents: ClipboardStoreData<
            Box<dyn Send + Sync + 'static + mime::AsMimeTypes>,
        >,
    ) {
    }
}

/// The kind of [`Clipboard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The standard clipboard.
    Standard,
    /// The primary clipboard.
    ///
    /// Normally only present in X11 and Wayland.
    Primary,
}

/// A null implementation of the [`Clipboard`] trait.
#[derive(Debug, Clone, Copy)]
pub struct Null;

impl Clipboard for Null {
    fn read(&self, _kind: Kind) -> Option<String> {
        None
    }

    fn write(&mut self, _kind: Kind, _contents: String) {}
}

/// Reads the current content of the [`Clipboard`].
pub fn read_data<T: AllowedMimeTypes>(
    kind: Kind,
    clipboard: &mut dyn Clipboard,
) -> Option<T> {
    clipboard
        .read_data(kind, T::allowed().into())
        .and_then(|data| T::try_from(data).ok())
}
