#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("device credential is not provisioned")]
    NotProvisioned,
    #[error("device request counter overflowed")]
    CounterOverflow,
    #[error("invalid provisioning challenge")]
    InvalidChallenge,
    #[error("invalid device key")]
    InvalidKey,
    #[error("identity I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("identity encoding failed: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("identity decoding failed: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("identity file is unsafe")]
    UnsafeIdentity,
}
