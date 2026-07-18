use core::{error::Error, fmt};

use super::ReserveError;

/// An error that can occur when converting a value to a [`LeanStr`].
///
/// This error can be caused by either a reserve error when allocating memory,
/// or a formatting error when converting the value to a string.
///
/// [`LeanStr`]: crate::LeanStr
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToLeanStrError {
    /// An error occurred while trying to allocate memory.
    Reserve(ReserveError),
    /// A formatting error occurred during conversion.
    Fmt(fmt::Error),
}

impl Error for ToLeanStrError {}

impl fmt::Display for ToLeanStrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToLeanStrError::Reserve(e) => e.fmt(f),
            ToLeanStrError::Fmt(e) => e.fmt(f),
        }
    }
}

impl From<ReserveError> for ToLeanStrError {
    fn from(value: ReserveError) -> Self {
        ToLeanStrError::Reserve(value)
    }
}

impl From<fmt::Error> for ToLeanStrError {
    fn from(value: fmt::Error) -> Self {
        ToLeanStrError::Fmt(value)
    }
}
