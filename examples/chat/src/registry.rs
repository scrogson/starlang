//! Room registry implementation using GenServer.
//!
//! The registry manages all chat rooms, creating them on demand
//! and providing lookups by name.

use crate::protocol::RoomInfo;
use crate::room::{Room, RoomInit};
use dream::gen_server::{self, prelude::*};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Registry GenServer implementation.
pub struct Registry;

/// Call requests to Registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegistryCall {
    /// Get a room by name.
    GetRoom(String),
    /// Get or create a room by name.
    GetOrCreateRoom(String),
    /// List all rooms with info.
    ListRooms,
}

/// Reply messages from Registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegistryReply {
    /// Room PID (None if not found).
    Room(Option<Pid>),
    /// List of room info.
    Rooms(Vec<RoomInfo>),
}

/// Cast messages to Registry (currently none).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegistryCast {}

// =============================================================================
// Client API
// =============================================================================

impl Registry {
    /// The registered name for the registry process.
    pub const NAME: &'static str = "registry";

    /// Start the registry GenServer.
    pub async fn start() -> Result<Pid, StartError> {
        gen_server::start::<Registry>(()).await
    }

    /// Get a room by name.
    pub async fn get_room(name: &str) -> Option<Pid> {
        match Self::call(RegistryCall::GetRoom(name.to_string())).await? {
            RegistryReply::Room(pid) => pid,
            _ => None,
        }
    }

    /// Get or create a room by name.
    pub async fn get_or_create_room(name: &str) -> Option<Pid> {
        match Self::call(RegistryCall::GetOrCreateRoom(name.to_string())).await? {
            RegistryReply::Room(pid) => pid,
            _ => None,
        }
    }

    /// List all rooms.
    pub async fn list_rooms() -> Vec<RoomInfo> {
        match Self::call(RegistryCall::ListRooms).await {
            Some(RegistryReply::Rooms(rooms)) => rooms,
            _ => vec![],
        }
    }

    /// Internal: make a call to the registry.
    async fn call(request: RegistryCall) -> Option<RegistryReply> {
        let registry_pid = dream::whereis(Self::NAME)?;

        match gen_server::call::<Registry>(registry_pid, request, Duration::from_secs(5)).await {
            Ok(reply) => Some(reply),
            Err(e) => {
                tracing::error!(error = ?e, "Registry call failed");
                None
            }
        }
    }
}

/// Registry state.
pub struct RegistryState {
    rooms: HashMap<String, Pid>,
}

#[async_trait]
impl GenServer for Registry {
    type State = RegistryState;
    type InitArg = ();
    type Call = RegistryCall;
    type Cast = RegistryCast;
    type Reply = RegistryReply;

    async fn init(_arg: ()) -> InitResult<RegistryState> {
        tracing::info!("Room registry started");
        InitResult::Ok(RegistryState {
            rooms: HashMap::new(),
        })
    }

    async fn handle_call(
        request: RegistryCall,
        _from: From,
        state: &mut RegistryState,
    ) -> CallResult<RegistryState, RegistryReply> {
        match request {
            RegistryCall::GetRoom(name) => {
                let pid = state.rooms.get(&name).copied();
                CallResult::reply(
                    RegistryReply::Room(pid),
                    RegistryState {
                        rooms: state.rooms.clone(),
                    },
                )
            }

            RegistryCall::GetOrCreateRoom(name) => {
                // Check if room exists
                if let Some(&pid) = state.rooms.get(&name) {
                    return CallResult::reply(
                        RegistryReply::Room(Some(pid)),
                        RegistryState {
                            rooms: state.rooms.clone(),
                        },
                    );
                }

                // Create new room - async callbacks let us await here!
                match gen_server::start::<Room>(RoomInit { name: name.clone() }).await
                {
                    Ok(pid) => {
                        tracing::info!(room = %name, pid = ?pid, "Room created");
                        state.rooms.insert(name, pid);
                        CallResult::reply(
                            RegistryReply::Room(Some(pid)),
                            RegistryState {
                                rooms: state.rooms.clone(),
                            },
                        )
                    }
                    Err(e) => {
                        tracing::error!(room = %name, error = ?e, "Failed to create room");
                        CallResult::reply(
                            RegistryReply::Room(None),
                            RegistryState {
                                rooms: state.rooms.clone(),
                            },
                        )
                    }
                }
            }

            RegistryCall::ListRooms => {
                let infos: Vec<RoomInfo> = state
                    .rooms
                    .keys()
                    .map(|name| RoomInfo {
                        name: name.clone(),
                        user_count: 0,
                    })
                    .collect();
                CallResult::reply(
                    RegistryReply::Rooms(infos),
                    RegistryState {
                        rooms: state.rooms.clone(),
                    },
                )
            }
        }
    }

    async fn handle_cast(_msg: RegistryCast, state: &mut RegistryState) -> CastResult<RegistryState> {
        CastResult::noreply(RegistryState {
            rooms: state.rooms.clone(),
        })
    }

    async fn handle_info(_msg: Vec<u8>, state: &mut RegistryState) -> InfoResult<RegistryState> {
        CastResult::noreply(RegistryState {
            rooms: state.rooms.clone(),
        })
    }

    async fn handle_continue(
        _arg: ContinueArg,
        state: &mut RegistryState,
    ) -> ContinueResult<RegistryState> {
        CastResult::noreply(RegistryState {
            rooms: state.rooms.clone(),
        })
    }
}
