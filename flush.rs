// Once a tick, at its end (the hook fires at the start and at the end).
fn on_tick_flush(game: &mut crate::game::Game<'_>, phase: horizon_client::mod_api::ServerTickPhase) {
    if !matches!(phase, horizon_client::mod_api::ServerTickPhase::ServerTickEnd { .. }) {
        return;
    }
    let players = game.native_server_players();
    // A fake player (a mod's bot with no client) has no voice chat.
    let guests: Vec<horizonvc_core::Guest> = players
        .iter()
        .filter(|p| !p.fake)
        .map(|p| horizonvc_core::Guest {
            uuid: p.uuid,
            name: &p.name,
            dimension: &p.dimension,
            pos: p.position,
            level: p.permission,
            has_mod: game
                .native_player_channels(p.uuid)
                .iter()
                .any(|c| c.starts_with("voicechat:")),
        })
        .collect();
    let out = horizonvc_core::tick(&guests);
    drop(guests);
    for o in out {
        match o {
            horizonvc_core::Out::Payload {
                player,
                channel,
                data,
            } => {
                if let Err(e) = game.native_send_custom_payload_to_player(player, channel, &data) {
                    log::warn!("[HorizonVC] {channel} not sent: {e}");
                }
            }
            horizonvc_core::Out::Chat { player, text } => {
                game.native_tell(player, horizon_client::mod_api::Text::literal(text));
            }
            horizonvc_core::Out::ChatCommand {
                player,
                text,
                command,
            } => {
                let line = horizon_client::mod_api::Text::literal(text.clone()).with_component(
                    serde_json::json!({
                        "text": text,
                        "color": "green",
                        "click_event": { "action": "run_command", "command": command },
                    }),
                );
                game.native_tell(player, line);
            }
            horizonvc_core::Out::Kick { player, text } => {
                game.native_kick(player, &horizon_client::mod_api::Text::literal(text));
            }
        }
    }
}
