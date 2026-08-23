mod error;
mod paths;

pub use error::RuntimeError;
pub use paths::{
    current_data_dir, current_runtime_dir, data_dir, prepare_data_dir, prepare_runtime_dir,
    runtime_dir, socket_path,
};
