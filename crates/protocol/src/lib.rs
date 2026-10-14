//! Version-specific wire formats for the supplied CODOL client.

pub const CLIENT_SHA256: &str = "e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b";

pub mod auth;
pub mod discovery;
pub mod game;
pub mod lobby;
pub mod room;
pub mod tls;
