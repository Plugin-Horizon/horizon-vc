//! HorizonVC: a voice chat server for Horizon that speaks the protocol of the Simple Voice Chat client mod, as a
//! Horizon native mod. The game half (two injections) hands plugin channel messages and the
//! players' positions to `server`; the voice itself runs on its own UDP thread.
//!
//! Clean-room: the formats and rules here were worked out from public documentation and from
//! the behaviour of the client mod on the wire (see the README).
pub mod config;
pub mod crypto;
pub mod packets;
pub mod routing;
pub mod server;
pub mod wire;

pub use server::{
    api, client_has_mod, command, on_payload, tick, Guest, Out, USAGES,
};
