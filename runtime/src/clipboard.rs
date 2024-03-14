//! Access the clipboard.
use window_clipboard::mime::{AllowedMimeTypes, AsMimeTypes};

use crate::command::{self, Command};
use crate::core::clipboard::Kind;
use crate::futures::MaybeSend;

use std::fmt;

/// A clipboard action to be performed by some [`Command`].
///
/// [`Command`]: crate::Command
pub enum Action<T> {
    /// Read the clipboard and produce `T` with the result.
    Read(Box<dyn Fn(Option<String>) -> T>, Kind),

    /// Write the given contents to the clipboard.
    Write(String, Kind),

    /// Write the given contents to the clipboard.
    WriteData(Box<dyn AsMimeTypes + Send + Sync + 'static>, Kind),

    #[allow(clippy::type_complexity)]
    /// Read the clipboard and produce `T` with the result.
    ReadData(
        Vec<String>,
        Box<dyn Fn(Option<(Vec<u8>, String)>) -> T>,
        Kind,
    ),
}

impl<T> Action<T> {
    /// Maps the output of a clipboard [`Action`] using the provided closure.
    pub fn map<A>(
        self,
        f: impl Fn(T) -> A + 'static + MaybeSend + Sync,
    ) -> Action<A>
    where
        T: 'static,
    {
        match self {
            Self::Read(o, target) => {
                Action::Read(Box::new(move |s| f(o(s))), target)
            }
            Self::Write(content, target) => Action::Write(content, target),
            Self::WriteData(content, target) => {
                Action::WriteData(content, target)
            }
            Self::ReadData(a, o, target) => {
                Action::ReadData(a, Box::new(move |s| f(o(s))), target)
            }
        }
    }
}

impl<T> fmt::Debug for Action<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(_, target) => write!(f, "Action::Read{target:?}"),
            Self::Write(_, target) => write!(f, "Action::Write({target:?})"),
            Self::WriteData(_, target) => {
                write!(f, "Action::WriteData({target:?})")
            }
            Self::ReadData(_, _, target) => {
                write!(f, "Action::ReadData({target:?})")
            }
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

/// Read the current contents of the primary clipboard.
pub fn read_primary<Message>(
    f: impl Fn(Option<String>) -> Message + 'static,
) -> Command<Message> {
    Command::single(command::Action::Clipboard(Action::Read(
        Box::new(f),
        Kind::Primary,
    )))
}

/// Write the given contents to the clipboard.
pub fn write<Message>(contents: String) -> Command<Message> {
    Command::single(command::Action::Clipboard(Action::Write(
        contents,
        Kind::Standard,
    )))
}

/// Write the given contents to the primary clipboard.
pub fn write_primary<Message>(contents: String) -> Command<Message> {
    Command::single(command::Action::Clipboard(Action::Write(
        contents,
        Kind::Primary,
    )))
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
