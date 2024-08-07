//! Access the clipboard.
use window_clipboard::mime::{AllowedMimeTypes, AsMimeTypes};
use crate::core::clipboard::Kind;
use crate::futures::futures::channel::oneshot;
use crate::Task;

/// A clipboard action to be performed by some [`Task`].
///
/// [`Task`]: crate::Task
#[derive(Debug)]
pub enum Action {
    /// Read the clipboard and produce `T` with the result.
    Read {
        /// The clipboard target.
        target: Kind,
        /// The channel to send the read contents.
        channel: oneshot::Sender<Option<String>>,
    },

    /// Write the given contents to the clipboard.
    Write {
        /// The clipboard target.
        target: Kind,
        /// The contents to be written.
        contents: String,
    },
    
    /// Write the given contents to the clipboard.
    WriteData{
        /// The contents to be written.
        contents: Box<dyn AsMimeTypes + Send + Sync + 'static>,
        /// The clipboard target.
        target: Kind
    },

    #[allow(clippy::type_complexity)]
    /// Read the clipboard and produce `T` with the result.
    ReadData{
        allowed_mimetypes: Vec<String>,
        o: Box<dyn Fn(Option<(Vec<u8>, String)>) -> T>,
        target: Kind,
    },
}

/// Read the current contents of the clipboard.
pub fn read() -> Task<Option<String>> {
    Task::oneshot(|channel| {
        crate::Action::Clipboard(Action::Read {
            target: Kind::Standard,
            channel,
        })
    })
}

/// Read the current contents of the primary clipboard.
pub fn read_primary() -> Task<Option<String>> {
    Task::oneshot(|channel| {
        crate::Action::Clipboard(Action::Read {
            target: Kind::Primary,
            channel,
        })
    })
}

/// Write the given contents to the clipboard.
pub fn write<T>(contents: String) -> Task<T> {
    Task::effect(crate::Action::Clipboard(Action::Write {
        target: Kind::Standard,
        contents,
    }))
}

/// Write the given contents to the primary clipboard.
pub fn write_primary<Message>(contents: String) -> Task<Message> {
    Task::effect(crate::Action::Clipboard(Action::Write {
        target: Kind::Primary,
        contents,
    }))
}

/// Read the current contents of the clipboard.
pub fn read_data<T: AllowedMimeTypes + Send + Sync + 'static, Message>(
    f: impl Fn(Option<T>) -> Message + 'static,
) -> Command<Message> {
    Command::single(command::Action::Clipboard(Action::ReadData(
        T::allowed().into(),
        Box::new(move |d| f(d.and_then(|d| T::try_from(d).ok()))),
        Kind::Standard,
    )))
}

/// Write the given contents to the clipboard.
pub fn write_data<Message>(
    contents: impl AsMimeTypes + std::marker::Sync + std::marker::Send + 'static,
) -> Command<Message> {
    Command::single(command::Action::Clipboard(Action::WriteData(
        Box::new(contents),
        Kind::Standard,
    )))
}

/// Read the current contents of the clipboard.
pub fn read_primary_data<
    T: AllowedMimeTypes + Send + Sync + 'static,
    Message,
>(
    f: impl Fn(Option<T>) -> Message + 'static,
) -> Command<Message> {
    Command::single(command::Action::Clipboard(Action::ReadData(
        T::allowed().into(),
        Box::new(move |d| f(d.and_then(|d| T::try_from(d).ok()))),
        Kind::Primary,
    )))
}

/// Write the given contents to the clipboard.
pub fn write_primary_data<Message>(
    contents: impl AsMimeTypes + std::marker::Sync + std::marker::Send + 'static,
) -> Command<Message> {
    Command::single(command::Action::Clipboard(Action::WriteData(
        Box::new(contents),
        Kind::Primary,
    )))
}
