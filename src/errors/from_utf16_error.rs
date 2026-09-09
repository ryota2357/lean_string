use core::{error::Error, fmt};

/// An error that can occur when converting a sequence of UTF-16 code units to a [`LeanString`] or
/// a [`LeanStr`].
///
/// This error can be caused by either a lone surrogate in the sequence, or, for the UTF-16LE and
/// UTF-16BE conversions, a byte slice with an odd number of bytes.
///
/// [`LeanString`]: crate::LeanString
/// [`LeanStr`]: crate::LeanStr
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FromUtf16Error {
    pub(crate) kind: FromUtf16ErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FromUtf16ErrorKind {
    LoneSurrogate,
    OddBytes,
}

impl Error for FromUtf16Error {}

impl fmt::Display for FromUtf16Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            FromUtf16ErrorKind::LoneSurrogate => "invalid utf-16: lone surrogate found",
            FromUtf16ErrorKind::OddBytes => "invalid utf-16: odd number of bytes",
        }
        .fmt(f)
    }
}
