//! Version-specific formats supported by evidence from the supplied client.

pub mod auth;
pub mod bootstrap;
pub mod lobby;
pub mod profile;
pub mod publisher;
pub mod stun;
pub mod task;
pub mod ticket;
pub mod tls;

pub const CLIENT_SHA256: &str = "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b";
