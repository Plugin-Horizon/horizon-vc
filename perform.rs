fn horizonvc_perform(
    game: &mut crate::game::Game<'_>,
    player: &crate::game::native_server::NativeServerPlayer,
    command: &str,
    ran: &mut bool,
) {
    if let Some(lines) = horizonvc_core::command(player.uuid, command) {
        *ran = true;
        for line in lines {
            game.native_feedback(horizon_client::text::Text::literal(line));
        }
    }
}
