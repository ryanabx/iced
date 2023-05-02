//! Access the clipboard.
pub use iced_runtime::clipboard::Action;

use iced_runtime::command::{self, Command};
use iced_style::core::clipboard::Kind;
use raw_window_handle::HasDisplayHandle;
use window_clipboard::mime::{self, ClipboardStoreData};

/// A buffer for short-term storage and transfer within and between
/// applications.
#[allow(missing_debug_implementations)]
pub struct Clipboard {
    pub(crate) state: State,
}

pub(crate) enum State {
    Connected(window_clipboard::Clipboard),
    Unavailable,
}

impl Clipboard {
    pub unsafe fn connect(display: &impl HasDisplayHandle) -> Clipboard {
        let context = window_clipboard::Clipboard::connect(display);

        Clipboard {
            state: context.map(State::Connected).unwrap_or(State::Unavailable),
        }
    }

    pub(crate) fn state(&self) -> &State {
        &self.state
    }

    /// Creates a new [`Clipboard`] that isn't associated with a window.
    /// This clipboard will never contain a copied value.
    pub fn unconnected() -> Clipboard {
        Clipboard {
            state: State::Unavailable,
        }
    }
}

impl iced_runtime::core::clipboard::Clipboard for Clipboard {
    fn read(&self, kind: Kind) -> Option<String> {
        match kind {
            Kind::Primary => match &self.state {
                State::Connected(clipboard) => {
                    clipboard.read_primary().and_then(|res| res.ok())
                }
                State::Unavailable => None,
            },
            Kind::Standard => match &self.state {
                State::Connected(clipboard) => clipboard.read().ok(),
                State::Unavailable => None,
            },
        }
    }

    fn write(&mut self, kind: Kind, contents: String) {
        match kind {
            Kind::Primary => match &mut self.state {
                State::Connected(clipboard) => {
                    _ = clipboard.write_primary(contents)
                }
                State::Unavailable => {}
            },
            Kind::Standard => match &mut self.state {
                State::Connected(clipboard) => _ = clipboard.write(contents),
                State::Unavailable => {}
            },
        }
    }

    fn read_data(
        &self,
        kind: Kind,
        mimes: Vec<String>,
    ) -> Option<(Vec<u8>, String)> {
        match kind {
            Kind::Primary => match &self.state {
                State::Connected(clipboard) => {
                    clipboard.read_primary_raw(mimes).and_then(|res| res.ok())
                }
                State::Unavailable => None,
            },
            Kind::Standard => match &self.state {
                State::Connected(clipboard) => {
                    clipboard.read_raw(mimes).and_then(|res| res.ok())
                }
                State::Unavailable => None,
            },
        }
    }

    fn write_data(
        &mut self,
        kind: Kind,
        contents: ClipboardStoreData<
            Box<dyn Send + Sync + 'static + mime::AsMimeTypes>,
        >,
    ) {
        match kind {
            Kind::Primary => match &mut self.state {
                State::Connected(clipboard) => {
                    _ = clipboard.write_primary_data(contents)
                }
                State::Unavailable => {}
            },
            Kind::Standard => match &mut self.state {
                State::Connected(clipboard) => {
                    _ = clipboard.write_data(contents)
                }
                State::Unavailable => {}
            },
        }
    }
}

/// Read the current contents of the clipboard.
pub fn read<Message>(
    f: impl Fn(Option<String>) -> Message + 'static,
) -> Command<Message> {
    Command::single(command::Action::Clipboard(Action::Read(
        Box::new(f),
        Kind::Standard,
    )))
}

/// Write the given contents to the clipboard.
pub fn write<Message>(contents: String) -> Command<Message> {
    Command::single(command::Action::Clipboard(Action::Write(
        contents,
        Kind::Standard,
    )))
}
