//! The original plugin's files, as fixtures: the port reads them with the same loaders the server uses.
use mod_voicechat::horizonvc_core::config::{Config, ORIGINAL_KEYS};
use mod_voicechat::horizonvc_core::translations::Texts;
use std::path::Path;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
fn every_one_of_the_19_original_keys_is_read() {
    assert_eq!(ORIGINAL_KEYS.len(), 19);
    let c = Config::read_original(&fixture("voicechat-server.properties")).unwrap();
    assert_eq!(c.port, 24460);
    assert_eq!(c.bind_address, "*");
    assert_eq!(c.max_voice_distance, 32.0);
    assert_eq!(c.whisper_distance, 12.0, "colon separator");
    assert_eq!(c.codec, "AUDIO");
    assert_eq!(c.mtu_size, 900);
    assert_eq!(c.tcp_rate_limit, 8);
    assert_eq!(c.keep_alive, 750);
    assert!(!c.enable_groups);
    assert_eq!(c.voice_host, "voice.example.invalid:24460");
    assert!(!c.allow_recording);
    assert!(c.spectator_interaction);
    assert!(c.spectator_player_possession);
    assert!(c.force_voice_chat);
    assert_eq!(c.login_timeout, 4000);
    assert_eq!(c.broadcast_range, -1.0);
    assert!(!c.allow_pings);
    assert!(!c.threaded_server_support);
}

#[test]
fn the_translations_fixture_is_read_with_its_escapes() {
    let t = Texts::load(&fixture("translations.properties"));
    assert_eq!(t.force_kick(), "Install Simple Voice Chat 2.6.27 first");
    assert_eq!(t.rate_limit_kick(), "Slow down, too many packets");
    assert_eq!(t.incompatible(), "Old client.\nPlease install version 2.6.27 of Simple Voice Chat.");
    assert_eq!(t.command_needed(), "Needs Simple Voice Chat on the client");
}

#[test]
fn a_missing_translations_file_gives_the_defaults() {
    assert_eq!(Texts::load(&fixture("no-such-file.properties")), Texts::default());
}
