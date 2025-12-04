//! GenFsm trait and server implementation.

use super::error::{CallError, SendEventError, StartError};
use super::protocol::FsmMessage;
use super::types::{CallResult, EventResult, From, InitResult};
use crate::core::{ExitReason, Pid, Ref, Term};
use async_trait::async_trait;
use std::time::Duration;
use tokio::sync::oneshot;

/// Generic Finite State Machine trait.
///
/// Implement this trait to create a process-based state machine.
///
/// # Type Parameters
///
/// - `State`: The enum representing possible states
/// - `Event`: The enum representing events that trigger transitions
/// - `Data`: Mutable data carried across state transitions
/// - `InitArg`: Argument passed to `init`
/// - `Call`: Request type for synchronous calls
/// - `Reply`: Response type for synchronous calls
#[async_trait]
pub trait GenFsm: Sized + Send + 'static {
    /// The state enum type.
    type State: Clone + Send + Sync + 'static;
    /// The event type that triggers state transitions.
    type Event: Term;
    /// Mutable data carried across states.
    type Data: Send + 'static;
    /// Argument passed to init.
    type InitArg: Send + 'static;
    /// Request type for synchronous calls.
    type Call: Term;
    /// Reply type for synchronous calls.
    type Reply: Term;

    /// Initialize the FSM with initial state and data.
    async fn init(arg: Self::InitArg) -> InitResult<Self::State, Self::Data>;

    /// Handle an event in the current state.
    ///
    /// This is the core state transition logic. Pattern match on `(state, event)`
    /// to implement your state machine transitions.
    async fn handle_event(
        state: &Self::State,
        event: Self::Event,
        data: &mut Self::Data,
    ) -> EventResult<Self::State, Self::Reply>;

    /// Handle a synchronous call.
    ///
    /// Override this to support request/response operations on the FSM.
    /// The default implementation keeps state without replying.
    async fn handle_call(
        _state: &Self::State,
        _request: Self::Call,
        _from: From,
        _data: &mut Self::Data,
    ) -> CallResult<Self::State, Self::Reply> {
        CallResult::keep_state()
    }

    /// Called when entering a new state.
    ///
    /// Override for side effects on state entry.
    async fn enter_state(_state: &Self::State, _data: &mut Self::Data) {}

    /// Called when exiting a state.
    ///
    /// Override for cleanup on state exit.
    async fn exit_state(_state: &Self::State, _data: &mut Self::Data) {}

    /// Called when the FSM is terminating.
    async fn terminate(_reason: ExitReason, _state: &Self::State, _data: &mut Self::Data) {}
}

/// Start a GenFsm process.
pub async fn start<F: GenFsm>(arg: F::InitArg) -> Result<Pid, StartError> {
    let (tx, rx) = oneshot::channel();

    let handle = crate::process::global::handle();
    let pid = handle.spawn(move || async move {
        run_fsm::<F>(arg, tx).await;
    });

    // Wait for init result
    match rx.await {
        Ok(Ok(())) => Ok(pid),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(StartError::SpawnFailed),
    }
}

/// Start a linked GenFsm process.
pub async fn start_link<F: GenFsm>(arg: F::InitArg) -> Result<Pid, StartError> {
    let (tx, rx) = oneshot::channel();

    let handle = crate::process::global::handle();
    let parent_pid = crate::runtime::current_pid();
    let pid = handle.spawn_link(parent_pid, move || async move {
        run_fsm::<F>(arg, tx).await;
    });

    match rx.await {
        Ok(Ok(())) => Ok(pid),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(StartError::SpawnFailed),
    }
}

/// Send an event to a GenFsm process (async, fire-and-forget).
pub fn send_event<F: GenFsm>(pid: Pid, event: F::Event) -> Result<(), SendEventError> {
    let msg = FsmMessage::Event(event.encode());
    let encoded = msg.encode();

    let handle = crate::process::global::handle();
    handle
        .registry()
        .send_raw(pid, encoded)
        .map_err(|_| SendEventError::ProcessNotFound(pid))
}

/// Make a synchronous call to a GenFsm process.
pub async fn call<F: GenFsm>(
    pid: Pid,
    request: F::Call,
    timeout: Duration,
) -> Result<F::Reply, CallError> {
    let reference = Ref::new();
    let caller = crate::runtime::current_pid();

    let msg = FsmMessage::Call {
        from: caller,
        reference,
        request: request.encode(),
    };

    let handle = crate::process::global::handle();
    handle
        .registry()
        .send_raw(pid, msg.encode())
        .map_err(|_| CallError::ProcessNotFound(pid))?;

    // Wait for reply
    match crate::runtime::recv_timeout(timeout).await {
        Ok(Some(data)) => {
            // Decode the FsmMessage::Reply
            match FsmMessage::decode(&data) {
                Ok(FsmMessage::Reply { reply, .. }) => {
                    F::Reply::decode(&reply).map_err(|e| CallError::DecodeError(format!("{:?}", e)))
                }
                _ => Err(CallError::DecodeError("unexpected message type".into())),
            }
        }
        Ok(None) => Err(CallError::Stopped(ExitReason::Normal)),
        Err(()) => Err(CallError::Timeout),
    }
}

/// Internal: Run the FSM process loop.
async fn run_fsm<F: GenFsm>(arg: F::InitArg, init_tx: oneshot::Sender<Result<(), StartError>>) {
    // Initialize
    let (mut state, mut data) = match F::init(arg).await {
        InitResult::Ok(s, d) => {
            let _ = init_tx.send(Ok(()));
            (s, d)
        }
        InitResult::OkTimeout(s, d, _timeout) => {
            let _ = init_tx.send(Ok(()));
            // TODO: Schedule state timeout
            (s, d)
        }
        InitResult::Ignore => {
            let _ = init_tx.send(Err(StartError::Ignore));
            return;
        }
        InitResult::Stop(reason) => {
            let _ = init_tx.send(Err(StartError::Stop(reason)));
            return;
        }
    };

    // Call enter_state for initial state
    F::enter_state(&state, &mut data).await;

    // Main loop
    loop {
        let msg_data = match crate::runtime::recv().await {
            Some(data) => data,
            None => break, // Mailbox closed
        };

        let msg = match FsmMessage::decode(&msg_data) {
            Ok(m) => m,
            Err(_) => continue, // Skip malformed messages
        };

        match msg {
            FsmMessage::Event(event_data) => {
                let event = match F::Event::decode(&event_data) {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                match F::handle_event(&state, event, &mut data).await {
                    EventResult::NextState(new_state) => {
                        F::exit_state(&state, &mut data).await;
                        state = new_state;
                        F::enter_state(&state, &mut data).await;
                    }
                    EventResult::NextStateTimeout(new_state, _timeout) => {
                        F::exit_state(&state, &mut data).await;
                        state = new_state;
                        F::enter_state(&state, &mut data).await;
                        // TODO: Schedule state timeout
                    }
                    EventResult::KeepState => {
                        // Stay in current state
                    }
                    EventResult::KeepStateTimeout(_timeout) => {
                        // TODO: Schedule state timeout
                    }
                    EventResult::NextStateActions(new_state, _actions) => {
                        F::exit_state(&state, &mut data).await;
                        state = new_state;
                        F::enter_state(&state, &mut data).await;
                        // TODO: Process actions
                    }
                    EventResult::KeepStateActions(_actions) => {
                        // TODO: Process actions
                    }
                    EventResult::Stop(reason) => {
                        F::terminate(reason, &state, &mut data).await;
                        break;
                    }
                    EventResult::StopReply(reason, from, reply) => {
                        send_reply(from, &reply);
                        F::terminate(reason, &state, &mut data).await;
                        break;
                    }
                }
            }
            FsmMessage::Call {
                from,
                reference,
                request,
            } => {
                let req = match F::Call::decode(&request) {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                let from_handle = From::new(from, reference);

                match F::handle_call(&state, req, from_handle, &mut data).await {
                    CallResult::Reply(reply, new_state) => {
                        send_reply(from_handle, &reply);
                        if !states_equal(&state, &new_state) {
                            F::exit_state(&state, &mut data).await;
                            state = new_state;
                            F::enter_state(&state, &mut data).await;
                        }
                    }
                    CallResult::ReplyTimeout(reply, new_state, _timeout) => {
                        send_reply(from_handle, &reply);
                        if !states_equal(&state, &new_state) {
                            F::exit_state(&state, &mut data).await;
                            state = new_state;
                            F::enter_state(&state, &mut data).await;
                        }
                        // TODO: Schedule state timeout
                    }
                    CallResult::NoReply(new_state) => {
                        if !states_equal(&state, &new_state) {
                            F::exit_state(&state, &mut data).await;
                            state = new_state;
                            F::enter_state(&state, &mut data).await;
                        }
                    }
                    CallResult::NoReplyTimeout(new_state, _timeout) => {
                        if !states_equal(&state, &new_state) {
                            F::exit_state(&state, &mut data).await;
                            state = new_state;
                            F::enter_state(&state, &mut data).await;
                        }
                        // TODO: Schedule state timeout
                    }
                    CallResult::KeepStateReply(reply) => {
                        send_reply(from_handle, &reply);
                    }
                    CallResult::KeepState => {
                        // Stay in current state, no reply
                    }
                    CallResult::Stop(reason, reply, new_state) => {
                        send_reply(from_handle, &reply);
                        state = new_state;
                        F::terminate(reason, &state, &mut data).await;
                        break;
                    }
                    CallResult::StopNoReply(reason, new_state) => {
                        state = new_state;
                        F::terminate(reason, &state, &mut data).await;
                        break;
                    }
                }
            }
            FsmMessage::Reply { .. } => {
                // Replies are handled by call() directly, shouldn't reach here
            }
            FsmMessage::StateTimeout => {
                // TODO: Generate internal timeout event
            }
            FsmMessage::GenericTimeout(_name) => {
                // TODO: Generate internal timeout event
            }
        }
    }

    F::terminate(ExitReason::Normal, &state, &mut data).await;
}

/// Send a reply to a caller.
fn send_reply<R: Term>(from: From, reply: &R) {
    let msg = FsmMessage::Reply {
        reference: from.reference,
        reply: reply.encode(),
    };

    let handle = crate::process::global::handle();
    let _ = handle.registry().send_raw(from.caller, msg.encode());
}

/// Compare two states for equality (used to determine if enter/exit should be called).
/// This is a simple pointer comparison fallback - ideally State implements PartialEq.
fn states_equal<S>(_a: &S, _b: &S) -> bool {
    // Conservative: always assume state changed
    // In practice, users can implement PartialEq on their State enum
    false
}
