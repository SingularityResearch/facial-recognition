//! The recognition core every platform shares: pictures, face finding, alignment, face prints, quality,
//! expression, and who a print belongs to. No window, no network, no files of its own beyond the models and
//! the library it is pointed at, so it builds the same for Windows, macOS, Linux, a Raspberry Pi, iOS and
//! Android. What it computes is held to fixed answers by `tests/parity.rs`.

pub mod behind;
pub mod faces;
pub mod gallery;
pub mod geometry;
pub mod image;
pub mod npy;
pub mod runtime;
pub mod voice;
pub mod yunet;

use std::fmt;

/// What went wrong, in words a log line or a page can show.
#[derive(Debug, Clone)]
pub struct Error(pub String);

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn msg(text: impl Into<String>) -> Error {
        Error(text.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error(e.to_string())
    }
}

impl<R> From<ort::Error<R>> for Error {
    fn from(e: ort::Error<R>) -> Error {
        Error(e.to_string())
    }
}
