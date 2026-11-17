fn horizonvc_payload(player: u128, channel: &str, data: &[u8]) {
    if channel.starts_with("voicechat:") {
        horizonvc_core::on_payload(player, channel, data);
    }
}
