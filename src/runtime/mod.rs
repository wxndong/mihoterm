mod config;
mod error;
mod process;

pub use error::RuntimeError;
pub use process::{ManagedRuntime, default_mihomo_executable, exec_managed_child};
