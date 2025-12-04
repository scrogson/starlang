//! # GenFsm - Generic Finite State Machine
//!
//! A process-based finite state machine pattern for Starlang, inspired by
//! Erlang's `gen_statem` but with Rust's type safety.
//!
//! # Overview
//!
//! GenFsm provides:
//! - **Enum-based states**: States are variants of an enum, enabling pattern matching
//! - **Event-driven transitions**: State changes happen in response to events
//! - **State-specific handlers**: Different behavior per state
//! - **Mutable data**: Carry data across state transitions
//! - **Process integration**: Runs as a supervised Starlang process
//!
//! # Example: Traffic Light
//!
//! ```ignore
//! use starlang::gen_fsm::{GenFsm, InitResult, EventResult, async_trait};
//! use serde::{Serialize, Deserialize};
//!
//! struct TrafficLight;
//!
//! #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
//! enum LightState {
//!     Red,
//!     Yellow,
//!     Green,
//! }
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! enum LightEvent {
//!     Timer,
//!     EmergencyStop,
//! }
//!
//! #[derive(Debug, Default)]
//! struct LightData {
//!     cycle_count: u32,
//! }
//!
//! #[async_trait]
//! impl GenFsm for TrafficLight {
//!     type State = LightState;
//!     type Event = LightEvent;
//!     type Data = LightData;
//!     type InitArg = ();
//!     type Reply = ();
//!
//!     async fn init(_arg: ()) -> InitResult<LightState, LightData> {
//!         InitResult::ok(LightState::Red, LightData::default())
//!     }
//!
//!     async fn handle_event(
//!         state: &LightState,
//!         event: LightEvent,
//!         data: &mut LightData,
//!     ) -> EventResult<LightState, ()> {
//!         match (state, event) {
//!             (LightState::Red, LightEvent::Timer) => {
//!                 EventResult::next_state(LightState::Green)
//!             }
//!             (LightState::Green, LightEvent::Timer) => {
//!                 EventResult::next_state(LightState::Yellow)
//!             }
//!             (LightState::Yellow, LightEvent::Timer) => {
//!                 data.cycle_count += 1;
//!                 EventResult::next_state(LightState::Red)
//!             }
//!             (_, LightEvent::EmergencyStop) => {
//!                 EventResult::next_state(LightState::Red)
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! # State Entry/Exit Actions
//!
//! Override `enter_state` and `exit_state` for side effects on transitions:
//!
//! ```ignore
//! async fn enter_state(state: &LightState, data: &mut LightData) {
//!     match state {
//!         LightState::Red => println!("STOP!"),
//!         LightState::Yellow => println!("CAUTION!"),
//!         LightState::Green => println!("GO!"),
//!     }
//! }
//! ```

#![deny(warnings)]
#![deny(missing_docs)]

mod error;
mod protocol;
mod server;
mod types;

pub use async_trait::async_trait;
pub use error::{CallError, SendEventError, StartError};
pub use server::{GenFsm, call, send_event, start, start_link};
pub use types::{CallResult, EventResult, From, InitResult, StateAction};

// Re-export commonly used types
pub use crate::core::{ExitReason, Pid, Term};

/// Prelude module for convenient imports.
pub mod prelude {
    pub use super::{
        CallError, CallResult, EventResult, ExitReason, From, GenFsm, InitResult, Pid,
        SendEventError, StartError, StateAction, async_trait, call, send_event, start, start_link,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use tokio::time::sleep;

    // Traffic Light FSM for testing
    struct TrafficLight;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    enum LightState {
        Red,
        Yellow,
        Green,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    enum LightEvent {
        Timer,
        Emergency,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    enum LightCall {
        GetState,
        GetCycleCount,
    }

    #[derive(Debug, Default)]
    struct LightData {
        cycle_count: u32,
    }

    #[async_trait]
    impl GenFsm for TrafficLight {
        type State = LightState;
        type Event = LightEvent;
        type Data = LightData;
        type InitArg = ();
        type Call = LightCall;
        type Reply = LightReply;

        async fn init(_arg: ()) -> InitResult<LightState, LightData> {
            InitResult::ok(LightState::Red, LightData::default())
        }

        async fn handle_event(
            state: &LightState,
            event: LightEvent,
            data: &mut LightData,
        ) -> EventResult<LightState, LightReply> {
            match (state, event) {
                (LightState::Red, LightEvent::Timer) => EventResult::next_state(LightState::Green),
                (LightState::Green, LightEvent::Timer) => {
                    EventResult::next_state(LightState::Yellow)
                }
                (LightState::Yellow, LightEvent::Timer) => {
                    data.cycle_count += 1;
                    EventResult::next_state(LightState::Red)
                }
                (_, LightEvent::Emergency) => EventResult::next_state(LightState::Red),
            }
        }

        async fn handle_call(
            state: &LightState,
            request: LightCall,
            _from: From,
            data: &mut LightData,
        ) -> CallResult<LightState, LightReply> {
            match request {
                LightCall::GetState => CallResult::reply(LightReply::State(*state), *state),
                LightCall::GetCycleCount => {
                    CallResult::reply(LightReply::Count(data.cycle_count), *state)
                }
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    enum LightReply {
        State(LightState),
        Count(u32),
    }

    #[tokio::test]
    async fn test_fsm_start() {
        crate::process::global::init();
        let handle = crate::process::global::handle();

        let pid = start::<TrafficLight>(()).await.unwrap();
        assert!(handle.alive(pid));

        sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_fsm_state_transitions() {
        crate::process::global::init();
        let handle = crate::process::global::handle();

        let pid = start::<TrafficLight>(()).await.unwrap();

        // Helper to get current state via call
        let get_state = Arc::new(AtomicU32::new(0)); // 0=Red, 1=Yellow, 2=Green

        // Initial state should be Red
        {
            let get_state = get_state.clone();
            handle.spawn(move || async move {
                if let Ok(LightReply::State(state)) =
                    call::<TrafficLight>(pid, LightCall::GetState, Duration::from_secs(5)).await
                {
                    let val = match state {
                        LightState::Red => 0,
                        LightState::Yellow => 1,
                        LightState::Green => 2,
                    };
                    get_state.store(val, Ordering::SeqCst);
                }
            });
        }
        sleep(Duration::from_millis(50)).await;
        assert_eq!(get_state.load(Ordering::SeqCst), 0); // Red

        // Send Timer event: Red -> Green
        send_event::<TrafficLight>(pid, LightEvent::Timer).unwrap();
        sleep(Duration::from_millis(50)).await;

        {
            let get_state = get_state.clone();
            handle.spawn(move || async move {
                if let Ok(LightReply::State(state)) =
                    call::<TrafficLight>(pid, LightCall::GetState, Duration::from_secs(5)).await
                {
                    let val = match state {
                        LightState::Red => 0,
                        LightState::Yellow => 1,
                        LightState::Green => 2,
                    };
                    get_state.store(val, Ordering::SeqCst);
                }
            });
        }
        sleep(Duration::from_millis(50)).await;
        assert_eq!(get_state.load(Ordering::SeqCst), 2); // Green
    }

    #[tokio::test]
    async fn test_fsm_data_persistence() {
        crate::process::global::init();
        let handle = crate::process::global::handle();

        let pid = start::<TrafficLight>(()).await.unwrap();

        // Complete one full cycle: Red -> Green -> Yellow -> Red
        send_event::<TrafficLight>(pid, LightEvent::Timer).unwrap(); // -> Green
        sleep(Duration::from_millis(20)).await;
        send_event::<TrafficLight>(pid, LightEvent::Timer).unwrap(); // -> Yellow
        sleep(Duration::from_millis(20)).await;
        send_event::<TrafficLight>(pid, LightEvent::Timer).unwrap(); // -> Red (cycle++)
        sleep(Duration::from_millis(50)).await;

        // Check cycle count
        let cycle_count = Arc::new(AtomicU32::new(0));
        {
            let cycle_count = cycle_count.clone();
            handle.spawn(move || async move {
                if let Ok(LightReply::Count(count)) =
                    call::<TrafficLight>(pid, LightCall::GetCycleCount, Duration::from_secs(5))
                        .await
                {
                    cycle_count.store(count, Ordering::SeqCst);
                }
            });
        }
        sleep(Duration::from_millis(50)).await;
        assert_eq!(cycle_count.load(Ordering::SeqCst), 1);
    }
}
