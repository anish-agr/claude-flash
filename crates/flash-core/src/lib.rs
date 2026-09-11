//! Platform-independent core of Claude Flash.
//!
//! Everything that decides behaviour lives here and is free of I/O: the policy
//! engine, configuration, Claude Code settings editing, the journal format, and the
//! request handling of the local HTTP API. The platform crate supplies sockets,
//! windows and clocks.

#![forbid(unsafe_code)]

pub mod color;
pub mod config;
pub mod duration;
pub mod engine;
pub mod event;
pub mod glob;
pub mod http;
pub mod journal;
pub mod render;
pub mod settings;
pub mod stats;
pub mod time;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
