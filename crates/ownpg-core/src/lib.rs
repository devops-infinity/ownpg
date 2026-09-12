pub mod audit;
pub mod classify;
pub mod config;
pub mod connect;
pub mod engine;
pub mod error;
pub mod groups;
pub mod server;
pub mod shape;
pub mod tools;

pub use classify::statement_count;
pub use config::{Environment, Mode, Settings};
pub use error::{Error, ErrorId, ExitClass, Result};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
