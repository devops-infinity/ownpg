pub mod classify;
pub mod config;
pub mod connect;
pub mod error;

pub use classify::statement_count;
pub use config::{Environment, Mode, Settings};
pub use error::{Error, ErrorId, ExitClass, Result};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
