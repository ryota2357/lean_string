use crate::{
    LeanStr, LeanString, ReserveError, ToLeanStrError, ToLeanStringError, UnwrapWithMsg,
    repr::{Immutable, Mutable, Repr},
};
use alloc::string::String;
use castaway::{LifetimeFree, match_type};
use core::{fmt, fmt::Write, num::NonZero};

/// A trait for converting a value to a [`LeanString`].
pub trait ToLeanString {
    /// Converts the value to a [`LeanString`].
    ///
    /// # Panics
    ///
    /// Panics if conversion fails. For a non-panicking version, use [`try_to_lean_string`].
    ///
    /// [`try_to_lean_string`]: Self::try_to_lean_string
    fn to_lean_string(&self) -> LeanString {
        self.try_to_lean_string().unwrap_with_msg()
    }

    /// Attempts to convert the value to a [`LeanString`].
    ///
    /// # Errors
    ///
    /// Returns a [`ToLeanStringError`] if the conversion fails.
    fn try_to_lean_string(&self) -> Result<LeanString, ToLeanStringError>;
}

impl ToLeanString for str {
    fn try_to_lean_string(&self) -> Result<LeanString, ToLeanStringError> {
        let repr = Repr::from_str(self)?;
        Ok(LeanString(repr))
    }
}

// NOTE: the restriction of `castaway` is `T` must be Sized.
impl<T: fmt::Display> ToLeanString for T {
    fn try_to_lean_string(&self) -> Result<LeanString, ToLeanStringError> {
        let repr = match_type!(self, {
            &i8 as s => Repr::from_num(*s)?,
            &u8 as s => Repr::from_num(*s)?,
            &i16 as s => Repr::from_num(*s)?,
            &u16 as s => Repr::from_num(*s)?,
            &i32 as s => Repr::from_num(*s)?,
            &u32 as s => Repr::from_num(*s)?,
            &i64 as s => Repr::from_num(*s)?,
            &u64 as s => Repr::from_num(*s)?,
            &i128 as s => Repr::from_num(*s)?,
            &u128 as s => Repr::from_num(*s)?,
            &isize as s => Repr::from_num(*s)?,
            &usize as s => Repr::from_num(*s)?,

            &NonZero<i8> as s => Repr::from_num(*s)?,
            &NonZero<u8> as s => Repr::from_num(*s)?,
            &NonZero<i16> as s => Repr::from_num(*s)?,
            &NonZero<u16> as s => Repr::from_num(*s)?,
            &NonZero<i32> as s => Repr::from_num(*s)?,
            &NonZero<u32> as s => Repr::from_num(*s)?,
            &NonZero<i64> as s => Repr::from_num(*s)?,
            &NonZero<u64> as s => Repr::from_num(*s)?,
            &NonZero<i128> as s => Repr::from_num(*s)?,
            &NonZero<u128> as s => Repr::from_num(*s)?,
            &NonZero<isize> as s => Repr::from_num(*s)?,
            &NonZero<usize> as s => Repr::from_num(*s)?,

            &f32 as s => Repr::from_num(*s)?,
            &f64 as s => Repr::from_num(*s)?,

            &bool as s => Repr::from_bool(*s),
            &char as s => Repr::from_char(*s),

            &String as s => Repr::from_str(s.as_str())?,
            &LeanString as s => return Ok(s.clone()),
            &LeanStr as s => return Ok(s.clone().try_into_lean_string()?),

            s => return try_from_fmt(format_args!("{s}")),
        });
        Ok(LeanString(repr))
    }
}

// SAFETY:
// - `LeanString` is `'static`.
// - `LeanString` does not contain any lifetime parameter.
// These two conditions are also applied to `Repr` which is the only field of `LeanString`.
unsafe impl LifetimeFree for LeanString {}
unsafe impl LifetimeFree for Repr<Mutable> {}

/// A trait for converting a value to a [`LeanStr`].
pub trait ToLeanStr {
    /// Converts the value to a [`LeanStr`].
    ///
    /// # Panics
    ///
    /// Panics if conversion fails. For a non-panicking version, use [`try_to_lean_str`].
    ///
    /// [`try_to_lean_str`]: Self::try_to_lean_str
    fn to_lean_str(&self) -> LeanStr {
        self.try_to_lean_str().unwrap_with_msg()
    }

    /// Attempts to convert the value to a [`LeanStr`].
    ///
    /// # Errors
    ///
    /// Returns a [`ToLeanStrError`] if the conversion fails.
    fn try_to_lean_str(&self) -> Result<LeanStr, ToLeanStrError>;
}

impl ToLeanStr for str {
    fn try_to_lean_str(&self) -> Result<LeanStr, ToLeanStrError> {
        let repr = Repr::from_str(self)?;
        Ok(LeanStr(repr))
    }
}

// NOTE: the restriction of `castaway` is `T` must be Sized.
impl<T: fmt::Display> ToLeanStr for T {
    fn try_to_lean_str(&self) -> Result<LeanStr, ToLeanStrError> {
        let repr = match_type!(self, {
            &i8 as s => Repr::from_num(*s)?,
            &u8 as s => Repr::from_num(*s)?,
            &i16 as s => Repr::from_num(*s)?,
            &u16 as s => Repr::from_num(*s)?,
            &i32 as s => Repr::from_num(*s)?,
            &u32 as s => Repr::from_num(*s)?,
            &i64 as s => Repr::from_num(*s)?,
            &u64 as s => Repr::from_num(*s)?,
            &i128 as s => Repr::from_num(*s)?,
            &u128 as s => Repr::from_num(*s)?,
            &isize as s => Repr::from_num(*s)?,
            &usize as s => Repr::from_num(*s)?,

            &NonZero<i8> as s => Repr::from_num(*s)?,
            &NonZero<u8> as s => Repr::from_num(*s)?,
            &NonZero<i16> as s => Repr::from_num(*s)?,
            &NonZero<u16> as s => Repr::from_num(*s)?,
            &NonZero<i32> as s => Repr::from_num(*s)?,
            &NonZero<u32> as s => Repr::from_num(*s)?,
            &NonZero<i64> as s => Repr::from_num(*s)?,
            &NonZero<u64> as s => Repr::from_num(*s)?,
            &NonZero<i128> as s => Repr::from_num(*s)?,
            &NonZero<u128> as s => Repr::from_num(*s)?,
            &NonZero<isize> as s => Repr::from_num(*s)?,
            &NonZero<usize> as s => Repr::from_num(*s)?,

            &f32 as s => Repr::from_num(*s)?,
            &f64 as s => Repr::from_num(*s)?,

            &bool as s => Repr::from_bool(*s),
            &char as s => Repr::from_char(*s),

            &String as s => Repr::from_str(s.as_str())?,
            &LeanStr as s => return Ok(s.clone()),
            &LeanString as s => return Ok(s.clone().try_into_lean_str()?),

            s => {
                let string = try_from_fmt::<ToLeanStrError>(format_args!("{s}"))?;
                return Ok(string.try_into_lean_str()?);
            }
        });
        Ok(LeanStr(repr))
    }
}

// SAFETY:
// - `LeanStr` is `'static`.
// - `LeanStr` does not contain any lifetime parameter.
// These two conditions are also applied to `Repr` which is the only field of `LeanStr`.
unsafe impl LifetimeFree for LeanStr {}
unsafe impl LifetimeFree for Repr<Immutable> {}

fn try_from_fmt<E>(args: fmt::Arguments<'_>) -> Result<LeanString, E>
where
    E: From<ReserveError> + From<fmt::Error>,
{
    if let Some(str) = args.as_str() {
        return Ok(LeanString::from_static_str(str));
    }

    struct Writer(Result<LeanString, ReserveError>);
    impl fmt::Write for Writer {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            let Ok(string) = &mut self.0 else { return Err(fmt::Error) };
            match string.try_push_str(s) {
                Ok(()) => Ok(()),
                Err(error) => {
                    self.0 = Err(error);
                    Err(fmt::Error)
                }
            }
        }
    }

    let mut writer = Writer(Ok(LeanString::new()));
    let formatted = writer.write_fmt(args);
    match (writer.0, formatted) {
        (Ok(string), Ok(())) => Ok(string),
        (Err(error), _) => Err(E::from(error)),
        (_, Err(error)) => Err(E::from(error)),
    }
}
