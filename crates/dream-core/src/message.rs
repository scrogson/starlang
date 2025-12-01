//! Message serialization trait.
//!
//! The [`Message`] trait provides a common interface for encoding and decoding
//! messages sent between processes. It uses `postcard` for compact binary
//! serialization.

use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

/// Error type for message decoding failures.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// Failed to deserialize the message bytes.
    #[error("failed to decode message: {0}")]
    Deserialize(#[from] postcard::Error),
}

/// A trait for types that can be sent as messages between processes.
///
/// This trait is automatically implemented for any type that implements
/// `Serialize + DeserializeOwned + Send + 'static`. The serialization uses
/// `postcard` for compact, efficient binary encoding.
///
/// # Examples
///
/// ```
/// use dream_core::Message;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// struct Ping {
///     id: u32,
/// }
///
/// let ping = Ping { id: 42 };
/// let bytes = ping.encode();
/// let decoded = Ping::decode(&bytes).unwrap();
/// assert_eq!(ping, decoded);
/// ```
pub trait Message: Sized + Send + 'static {
    /// Encodes this message into bytes.
    ///
    /// # Panics
    ///
    /// Panics if serialization fails. This should not happen for well-formed
    /// types that properly implement `Serialize`.
    fn encode(&self) -> Vec<u8>;

    /// Decodes a message from bytes.
    ///
    /// # Errors
    ///
    /// Returns `DecodeError` if the bytes cannot be deserialized into this type.
    fn decode(bytes: &[u8]) -> Result<Self, DecodeError>;

    /// Encodes this message, returning `None` on failure instead of panicking.
    fn try_encode(&self) -> Option<Vec<u8>>;
}

impl<T> Message for T
where
    T: Serialize + DeserializeOwned + Send + 'static,
{
    fn encode(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("message serialization failed")
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        postcard::from_bytes(bytes).map_err(DecodeError::from)
    }

    fn try_encode(&self) -> Option<Vec<u8>> {
        postcard::to_allocvec(self).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestMessage {
        id: u32,
        name: String,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    enum TestEnum {
        Ping(u64),
        Pong { value: String },
    }

    #[test]
    fn test_encode_decode_struct() {
        let msg = TestMessage {
            id: 42,
            name: "hello".to_string(),
        };
        let bytes = msg.encode();
        let decoded = TestMessage::decode(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_encode_decode_enum() {
        let msg = TestEnum::Ping(123);
        let bytes = msg.encode();
        let decoded = TestEnum::decode(&bytes).unwrap();
        assert_eq!(msg, decoded);

        let msg = TestEnum::Pong {
            value: "world".to_string(),
        };
        let bytes = msg.encode();
        let decoded = TestEnum::decode(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_decode_error() {
        let bad_bytes = vec![0xFF, 0xFF, 0xFF];
        let result = TestMessage::decode(&bad_bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_try_encode() {
        let msg = TestMessage {
            id: 1,
            name: "test".to_string(),
        };
        let bytes = msg.try_encode();
        assert!(bytes.is_some());
    }

    #[test]
    fn test_primitive_types() {
        // u32
        let num: u32 = 42;
        let bytes = num.encode();
        let decoded = u32::decode(&bytes).unwrap();
        assert_eq!(num, decoded);

        // String
        let s = "hello world".to_string();
        let bytes = s.encode();
        let decoded = String::decode(&bytes).unwrap();
        assert_eq!(s, decoded);

        // Vec<u8>
        let v: Vec<u8> = vec![1, 2, 3, 4];
        let bytes = v.encode();
        let decoded = Vec::<u8>::decode(&bytes).unwrap();
        assert_eq!(v, decoded);
    }

    #[test]
    fn test_option_types() {
        let some: Option<u32> = Some(42);
        let bytes = some.encode();
        let decoded = Option::<u32>::decode(&bytes).unwrap();
        assert_eq!(some, decoded);

        let none: Option<u32> = None;
        let bytes = none.encode();
        let decoded = Option::<u32>::decode(&bytes).unwrap();
        assert_eq!(none, decoded);
    }

    #[test]
    fn test_tuple_types() {
        let tuple: (u32, String, bool) = (42, "test".to_string(), true);
        let bytes = tuple.encode();
        let decoded = <(u32, String, bool)>::decode(&bytes).unwrap();
        assert_eq!(tuple, decoded);
    }

    #[test]
    fn test_unit_type() {
        let unit: () = ();
        let bytes = unit.encode();
        let decoded = <()>::decode(&bytes).unwrap();
        assert_eq!(unit, decoded);
    }
}
