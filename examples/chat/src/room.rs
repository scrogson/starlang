//! Chat room implementation using GenServer.
//!
//! Each room is a GenServer that manages its members and broadcasts messages.
//! Uses `pg` (process groups) for distributed room membership.

use crate::protocol::{RoomInfo, ServerEvent};
use dream::dist::pg;
use dream::gen_server::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Room GenServer implementation.
pub struct Room;

/// Room state.
pub struct RoomState {
    /// Room name.
    pub name: String,
    /// Members: PID -> nickname.
    /// This is the authoritative list of nicknames for members.
    pub members: HashMap<Pid, String>,
}

/// Initialization argument for Room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInit {
    pub name: String,
}

/// Call requests to Room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoomCall {
    /// Get room info.
    GetInfo,
    /// Get list of members.
    GetMembers,
}

/// Call replies from Room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoomReply {
    /// Room info.
    Info(RoomInfo),
    /// List of member nicknames.
    Members(Vec<String>),
}

/// Cast messages to Room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoomCast {
    /// A user joins the room.
    Join { pid: Pid, nick: String },
    /// A user leaves the room.
    Leave { pid: Pid },
    /// Broadcast a message from a user.
    Broadcast { from_pid: Pid, text: String },
    /// Update a user's nickname.
    UpdateNick { pid: Pid, new_nick: String },
}

/// Internal message type for sending events to user sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserEvent(pub ServerEvent);

#[async_trait]
impl GenServer for Room {
    type State = RoomState;
    type InitArg = RoomInit;
    type Call = RoomCall;
    type Cast = RoomCast;
    type Reply = RoomReply;

    async fn init(arg: RoomInit) -> InitResult<RoomState> {
        tracing::info!(room = %arg.name, "Room created");
        InitResult::Ok(RoomState {
            name: arg.name,
            members: HashMap::new(),
        })
    }

    async fn handle_call(
        request: RoomCall,
        _from: From,
        state: &mut RoomState,
    ) -> CallResult<RoomState, RoomReply> {
        match request {
            RoomCall::GetInfo => {
                let info = RoomInfo {
                    name: state.name.clone(),
                    user_count: state.members.len(),
                };
                let new_state = RoomState {
                    name: state.name.clone(),
                    members: state.members.clone(),
                };
                CallResult::Reply(RoomReply::Info(info), new_state)
            }
            RoomCall::GetMembers => {
                let members: Vec<String> = state.members.values().cloned().collect();
                let new_state = RoomState {
                    name: state.name.clone(),
                    members: state.members.clone(),
                };
                CallResult::Reply(RoomReply::Members(members), new_state)
            }
        }
    }

    async fn handle_cast(msg: RoomCast, state: &mut RoomState) -> CastResult<RoomState> {
        let group = room_group(&state.name);

        match msg {
            RoomCast::Join { pid, nick } => {
                tracing::info!(room = %state.name, nick = %nick, ?pid, "User joined");

                // Notify existing members (broadcast before adding new user)
                let event = ServerEvent::UserJoined {
                    room: state.name.clone(),
                    nick: nick.clone(),
                };
                broadcast_to_group(&group, &UserEvent(event));

                // Join the pg group for this room
                pg::join(&group, pid);
                state.members.insert(pid, nick);

                CastResult::NoReply(RoomState {
                    name: state.name.clone(),
                    members: state.members.clone(),
                })
            }
            RoomCast::Leave { pid } => {
                if let Some(nick) = state.members.remove(&pid) {
                    tracing::info!(room = %state.name, nick = %nick, "User left");

                    // Leave the pg group
                    pg::leave(&group, pid);

                    // Notify remaining members
                    let event = ServerEvent::UserLeft {
                        room: state.name.clone(),
                        nick,
                    };
                    broadcast_to_group(&group, &UserEvent(event));
                }

                CastResult::NoReply(RoomState {
                    name: state.name.clone(),
                    members: state.members.clone(),
                })
            }
            RoomCast::Broadcast { from_pid, text } => {
                if let Some(nick) = state.members.get(&from_pid) {
                    let event = ServerEvent::Message {
                        room: state.name.clone(),
                        from: nick.clone(),
                        text,
                    };
                    broadcast_to_group(&group, &UserEvent(event));
                }

                CastResult::NoReply(RoomState {
                    name: state.name.clone(),
                    members: state.members.clone(),
                })
            }
            RoomCast::UpdateNick { pid, new_nick } => {
                if let Some(nick) = state.members.get_mut(&pid) {
                    *nick = new_nick;
                }

                CastResult::NoReply(RoomState {
                    name: state.name.clone(),
                    members: state.members.clone(),
                })
            }
        }
    }

    async fn handle_info(_msg: Vec<u8>, state: &mut RoomState) -> InfoResult<RoomState> {
        InfoResult::NoReply(RoomState {
            name: state.name.clone(),
            members: state.members.clone(),
        })
    }

    async fn handle_continue(_arg: ContinueArg, state: &mut RoomState) -> ContinueResult<RoomState> {
        ContinueResult::NoReply(RoomState {
            name: state.name.clone(),
            members: state.members.clone(),
        })
    }
}

/// Generate the pg group name for a room.
fn room_group(room_name: &str) -> String {
    format!("room:{}", room_name)
}

/// Broadcast a message to all members of a pg group.
fn broadcast_to_group<M: Serialize>(group: &str, message: &M) {
    let members = pg::get_members(group);
    if let Ok(payload) = postcard::to_allocvec(message) {
        for pid in members {
            let _ = dream::send_raw(pid, payload.clone());
        }
    }
}
