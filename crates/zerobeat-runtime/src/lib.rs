mod error;
mod paths;

pub use error::RuntimeError;
pub use paths::{current_runtime_dir, prepare_runtime_dir, runtime_dir, socket_path};
