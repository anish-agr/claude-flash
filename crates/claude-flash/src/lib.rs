//! The Claude Flash agent and its command line.
//!
//! `flash-agent` runs in the background. Claude Code's hooks deliver events to it
//! over loopback HTTP, [`flash_core::engine`] decides what deserves attention, and
//! the platform front end in [`ui`] shows it. `flash` is the command line for all of
//! it.

pub mod agent;
pub mod api;
pub mod autostart;
pub mod cli;
pub mod client;
pub mod hub;
pub mod journal;
pub mod logging;
pub mod paths;
pub mod push;
pub mod runtime;
pub mod server;
pub mod store;
pub mod system;
pub mod ui;
