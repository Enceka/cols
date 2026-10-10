//! Version-specific formats supported by evidence from the supplied client.

pub mod auth;
pub mod bootstrap;
pub mod game;
pub mod game_corpse;
pub mod game_enemy;
pub mod game_events;
pub mod game_hit_feedback;
pub mod game_impact;
pub mod game_obituary;
pub mod game_scoreboard;
pub mod game_snapshot;
pub mod game_state;
pub mod game_stats;
pub mod game_target;
pub mod game_usercmd;
pub mod game_wire;
pub mod lobby;
pub mod profile;
pub mod publisher;
pub mod purchase;
pub mod room;
pub mod room_messages;
pub mod stun;
pub mod task;
pub mod ticket;
pub mod tls;

pub const CLIENT_SHA256: &str = "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b";
