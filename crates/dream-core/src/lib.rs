//! # dream-core
//!
//! Core types for DREAM (Distributed Rust Erlang Abstract Machine).
//!
//! This crate provides the foundational types used throughout the DREAM ecosystem:
//!
//! - [`Pid`] - Process identifier
//! - [`Ref`] - Unique reference for monitors and timers
//! - [`ExitReason`] - Process termination reasons
//! - [`Message`] - Trait for serializable messages
//! - [`SystemMessage`] - Internal system messages (Exit, Down, Timeout)

mod exit_reason;
mod message;
mod pid;
mod reference;
mod system_message;

pub use exit_reason::ExitReason;
pub use message::{DecodeError, Message};
pub use pid::Pid;
pub use reference::Ref;
pub use system_message::SystemMessage;
