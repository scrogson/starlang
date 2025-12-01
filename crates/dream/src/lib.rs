//! # DREAM - Distributed Rust Erlang Abstract Machine
//!
//! DREAM is a native Rust implementation of Erlang/OTP primitives, providing
//! type-safe processes, message passing, supervision trees, and application
//! lifecycle management.
//!
//! # Overview
//!
//! DREAM brings Erlang's battle-tested concurrency primitives to Rust:
//!
//! - **Processes**: Lightweight, isolated units of concurrency with mailboxes
//! - **Links**: Bidirectional failure propagation between processes
//! - **Monitors**: Unidirectional process observation
//! - **GenServer**: Generic server pattern for stateful processes
//! - **Supervisor**: Automatic process restart and fault tolerance
//! - **Application**: Top-level lifecycle management with dependencies
//!
//! # Quick Start
//!
//! ```ignore
//! use dream::prelude::*;
//!
//! // Create a runtime
//! let runtime = Runtime::new();
//! let handle = runtime.handle();
//!
//! // Spawn a process
//! let pid = handle.spawn(async move |ctx| {
//!     println!("Hello from process {:?}", ctx.self_pid());
//! });
//!
//! // Send a message
//! handle.send(pid, b"Hello!".to_vec());
//! ```
//!
//! # GenServer Example
//!
//! ```ignore
//! use dream::prelude::*;
//! use dream::gen_server::{GenServer, InitResult, CallResult, CastResult, InfoResult, ContinueResult, From, ContinueArg};
//! use serde::{Serialize, Deserialize};
//!
//! struct Counter;
//!
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! enum CounterCall {
//!     Get,
//!     Increment,
//! }
//!
//! impl GenServer for Counter {
//!     type State = i64;
//!     type InitArg = i64;
//!     type Call = CounterCall;
//!     type Cast = ();
//!     type Reply = i64;
//!
//!     fn init(initial: i64) -> InitResult<i64> {
//!         InitResult::Ok(initial)
//!     }
//!
//!     fn handle_call(request: CounterCall, _from: From, state: &mut i64) -> CallResult<i64, i64> {
//!         match request {
//!             CounterCall::Get => CallResult::Reply(*state, *state),
//!             CounterCall::Increment => {
//!                 *state += 1;
//!                 CallResult::Reply(*state, *state)
//!             }
//!         }
//!     }
//!
//!     fn handle_cast(_msg: (), state: &mut i64) -> CastResult<i64> {
//!         CastResult::NoReply(*state)
//!     }
//!
//!     fn handle_info(_msg: Vec<u8>, state: &mut i64) -> InfoResult<i64> {
//!         InfoResult::NoReply(*state)
//!     }
//!
//!     fn handle_continue(_arg: ContinueArg, state: &mut i64) -> ContinueResult<i64> {
//!         ContinueResult::NoReply(*state)
//!     }
//! }
//! ```
//!
//! # Supervisor Example
//!
//! ```ignore
//! use dream::prelude::*;
//! use dream::supervisor::{Supervisor, ChildSpec, Strategy, SupervisorFlags};
//!
//! struct MySupervisor;
//!
//! impl Supervisor for MySupervisor {
//!     fn init(handle: &RuntimeHandle) -> (SupervisorFlags, Vec<ChildSpec>) {
//!         let flags = SupervisorFlags::new(Strategy::OneForOne)
//!             .max_restarts(3)
//!             .max_seconds(5);
//!
//!         let children = vec![
//!             // Define child specs here
//!         ];
//!
//!         (flags, children)
//!     }
//! }
//! ```

#![deny(warnings)]
#![deny(missing_docs)]

// Re-export core types
pub use dream_core::{ExitReason, Message, Pid, Ref};

// Re-export runtime and process types
pub use dream_process::{Runtime, RuntimeHandle};
pub use dream_runtime::Context;

// Re-export GenServer
pub use dream_gen_server as gen_server;

// Re-export Supervisor
pub use dream_supervisor as supervisor;

// Re-export Application
pub use dream_application as application;

// Re-export macros
pub use dream_macros::{dream_process, GenServerImpl};

/// Prelude module for convenient imports.
///
/// Import everything commonly needed with:
/// ```ignore
/// use dream::prelude::*;
/// ```
pub mod prelude {
    // Core types
    pub use dream_core::{ExitReason, Message, Pid, Ref};

    // Runtime and process
    pub use dream_process::{Runtime, RuntimeHandle};
    pub use dream_runtime::Context;

    // GenServer essentials
    pub use dream_gen_server::{
        CallResult, CastResult, ContinueArg, ContinueResult, From, GenServer, InfoResult,
        InitResult, ServerRef,
    };

    // Supervisor essentials
    pub use dream_supervisor::{
        ChildSpec, ChildType, RestartType, ShutdownType, Strategy, Supervisor, SupervisorFlags,
    };

    // Application essentials
    pub use dream_application::{AppConfig, AppController, AppSpec, Application, StartResult};

    // Macros
    pub use dream_macros::{dream_process, GenServerImpl};
}

#[cfg(test)]
mod tests {
    use super::prelude::*;

    #[test]
    fn test_prelude_imports() {
        // This test verifies that all prelude imports compile
        let _pid: Option<Pid> = None;
        let _ref: Option<Ref> = None;
        let _reason = ExitReason::Normal;
    }

    #[tokio::test]
    async fn test_basic_spawn() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let runtime = Runtime::new();
        let handle = runtime.handle();

        let executed = Arc::new(AtomicBool::new(false));
        let executed_clone = executed.clone();

        let _pid = handle.spawn(move |_ctx| async move {
            executed_clone.store(true, Ordering::SeqCst);
        });

        // Give the process time to execute
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert!(executed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_gen_server_integration() {
        use serde::{Deserialize, Serialize};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        struct TestServer;

        #[derive(Debug, Clone, Serialize, Deserialize)]
        enum TestCall {
            Ping,
        }

        impl GenServer for TestServer {
            type State = ();
            type InitArg = ();
            type Call = TestCall;
            type Cast = ();
            type Reply = String;

            fn init(_: ()) -> InitResult<()> {
                InitResult::Ok(())
            }

            fn handle_call(
                request: TestCall,
                _from: From,
                state: &mut (),
            ) -> CallResult<(), String> {
                match request {
                    TestCall::Ping => CallResult::Reply("pong".to_string(), *state),
                }
            }

            fn handle_cast(_: (), state: &mut ()) -> CastResult<()> {
                CastResult::NoReply(*state)
            }

            fn handle_info(_: Vec<u8>, state: &mut ()) -> InfoResult<()> {
                InfoResult::NoReply(*state)
            }

            fn handle_continue(_: ContinueArg, state: &mut ()) -> ContinueResult<()> {
                ContinueResult::NoReply(*state)
            }
        }

        let runtime = Runtime::new();
        let handle = runtime.handle();

        // Start the server
        let server_pid = dream_gen_server::start::<TestServer>(&handle, ()).await.unwrap();

        // We need to call from within a process since `call` requires a Context
        let test_passed = Arc::new(AtomicBool::new(false));
        let test_passed_clone = test_passed.clone();
        let handle_clone = handle.clone();

        handle.spawn(move |ctx| async move {
            let mut ctx = ctx;
            let reply: String = dream_gen_server::call::<TestServer>(
                &handle_clone,
                &mut ctx,
                server_pid,
                TestCall::Ping,
                Duration::from_secs(5),
            )
            .await
            .unwrap();

            if reply == "pong" {
                test_passed_clone.store(true, Ordering::SeqCst);
            }
        });

        // Give the test time to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(test_passed.load(Ordering::SeqCst), "GenServer call should return 'pong'");
    }

    #[tokio::test]
    async fn test_application_integration() {
        struct TestApp;

        impl Application for TestApp {
            fn start(_: &RuntimeHandle, _: &AppConfig) -> Result<StartResult, String> {
                Ok(StartResult::None)
            }

            fn spec() -> AppSpec {
                AppSpec::new("test_app")
            }
        }

        let runtime = Runtime::new();
        let handle = runtime.handle();
        let controller = AppController::new(handle);

        controller.register::<TestApp>();
        controller.start("test_app").await.unwrap();
        assert!(controller.is_running("test_app"));

        controller.stop("test_app").await.unwrap();
        assert!(!controller.is_running("test_app"));
    }
}
