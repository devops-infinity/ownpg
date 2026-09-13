#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::indexing_slicing,
    dead_code
)]

mod support;

mod connect;
mod ddl;
mod engine;
mod host;
mod http;
mod ops;
mod pooler;
mod resources;
mod server;
mod ssh;
mod write;
