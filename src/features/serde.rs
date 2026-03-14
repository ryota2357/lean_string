use crate::LeanString;
use core::{fmt, str};
use serde_core::{
    de::{Deserialize, Deserializer, Error, Unexpected, Visitor},
    ser::{Serialize, Serializer},
};

#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
impl Serialize for LeanString {
    #[inline]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_str().serialize(serializer)
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
impl<'de> Deserialize<'de> for LeanString {
    #[inline]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct LeanStringVisitor;

        impl<'de> Visitor<'de> for LeanStringVisitor {
            type Value = LeanString;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(LeanString::from(v))
            }

            fn visit_borrowed_str<E: Error>(self, v: &'de str) -> Result<Self::Value, E> {
                Ok(LeanString::from(v))
            }

            fn visit_bytes<E: Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                LeanString::from_utf8(v)
                    .map_err(|_| Error::invalid_value(Unexpected::Bytes(v), &self))
            }

            fn visit_borrowed_bytes<E: Error>(self, v: &'de [u8]) -> Result<Self::Value, E> {
                LeanString::from_utf8(v)
                    .map_err(|_| Error::invalid_value(Unexpected::Bytes(v), &self))
            }
        }

        deserializer.deserialize_string(LeanStringVisitor)
    }
}
