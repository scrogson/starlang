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
//! - [`NodeId`], [`NodeName`], [`NodeInfo`] - Node identity for distribution

#![deny(warnings)]
#![deny(missing_docs)]

mod exit_reason;
mod message;
pub mod node;
mod pid;
mod reference;
mod system_message;

pub use exit_reason::ExitReason;
pub use message::{DecodeError, Message};
pub use node::{NodeId, NodeInfo, NodeName};
pub use pid::{current_creation, increment_creation, Pid};
pub use reference::Ref;
pub use system_message::SystemMessage;
