//! Voice chat's trait-mod half: the hook the mixin injections cannot reach, handed to the core.
//! `custom_payload_from_client` takes a player's `voicechat:*` plugin message (the other
//! channels are not ours). Which clients have the mod is read from `native_player_channels` in
//! the tick flush.
//!
//! Written against Horizon a589a3dd9 (the event fields follow `mod_api_generated.rs` of
//! the showcase build); not yet compiled.

use horizon_client::mod_api::{CustomPayloadFromClientEvent, Hooks, Host};

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(HorizonVc)
}

struct HorizonVc;

impl Hooks for HorizonVc {
    fn custom_payload_from_client(&self, _host: &mut dyn Host, event: &mut CustomPayloadFromClientEvent<'_>) {
        if event.channel.starts_with("voicechat:") {
            horizonvc_core::on_payload(event.player, event.channel, event.data);
        }
    }
}
