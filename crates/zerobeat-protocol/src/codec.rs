use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("failed to encode protocol message: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("failed to decode protocol message: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
}

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    rmp_serde::to_vec_named(value).map_err(ProtocolError::from)
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProtocolError> {
    rmp_serde::from_slice(bytes).map_err(ProtocolError::from)
}
