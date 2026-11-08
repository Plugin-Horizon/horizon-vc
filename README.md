# HorizonVC

A voice chat server for [Horizon](https://github.com/horizon-rust/horizon), the Rust server for
Minecraft 26.3, written as a native Horizon mod.

Players use the Simple Voice Chat client mod; HorizonVC implements a compatible server. Not affiliated with Simple Voice Chat.

The server hands out the voice secrets, runs the UDP voice port and routes audio by proximity and
groups. It speaks protocol compatibility version 20 (client mod 2.6.x).

## What it does

- Plugin channel handshake: announces its channels (`minecraft:register`), answers
  `voicechat:request_secret` with the player list (`voicechat:states`), the groups and
  `voicechat:secret` (port, codec, MTU, distance, keep-alive, groups on/off, voice host,
  recording allowed). A client with another compatibility version gets a short chat notice.
- Voice UDP port: authentication, connection check, keep-alives (ten missed ones end the voice
  connection; the client is sent its secret again to reconnect), AES-128-GCM per datagram.
- Proximity routing: a player's microphone packets go to every voice-connected player in the same
  dimension within `max_voice_distance` (+1 block), as player sounds carrying the normal or whisper
  distance. Players who turned their voice chat off hear nothing but are heard.
- Groups (`enable_groups`): create (with an optional password), join, leave; normal, open and
  isolated groups; group members hear each other at any distance; empty groups disappear when
  their last member leaves the group or the server.
- Player states: joined, voice connected/disconnected, disabled, group; removed on leave.
- Commands `/horizonvc` (alias `/hvc`): `help`, `test <player>` (operators), `invite <player>`
  (a clickable line that runs `/horizonvc join <group id>`; the invitation stands in for the
  password once), `join <group name> [password]` (refused when more than one group has that name),
  `join <group id>`, `leave`.
- `force_voice_chat` with `login_timeout`: a player whose client has not asked for a voice secret
  within the timeout is kicked.
- Permissions `horizonvc.listen`, `horizonvc.speak`, `horizonvc.groups`, `horizonvc.admin`
  (default: everyone; `admin` for operators). Horizon has no permission system, so another mod can
  answer them through `api::set_permission_check`. A player without `listen` hears nothing, one
  without `speak` is not routed, one without `groups` cannot create or join groups.
- A Rust API for other mods (`horizonvc_core::api`): `connection_of`, `groups`, `create_group`
  (with persistent groups, which stay when empty), `remove_group`, `set_group`,
  `set_permission_check`, `add_mic_listener` (hear or drop each microphone packet),
  `send_as_player` (Opus data routed as a player's voice, for players without the mod) and
  `send_static` (a static audio channel: one listener, as group audio).

## Names that must stay `voicechat:`

The client mod only talks on plugin channels named `voicechat:<name>` (`request_secret`, `secret`,
`states`, `state`, `remove_state`, `add_group`, `remove_group`, `joined_group`, `create_group`,
`set_group`, `leave_group`, `update_state`), and the server recognises a client with the mod by those
channels. These ids, the packet layouts and the compatibility number are part of the protocol and
are not renamed. Everything else is HorizonVC's own: command, permission nodes, config, log prefix.

## Config

`config/horizonvc/horizonvc-server.properties` in the server folder, written with the defaults on
the first start ([core/horizonvc-server.properties](core/horizonvc-server.properties)). Keys:
`port` (-1: the game port), `bind_address` (empty: `server-ip`, `*`: every address),
`max_voice_distance`, `whisper_distance`, `codec`, `mtu_size`, `tcp_rate_limit`, `keep_alive`,
`enable_groups`, `voice_host`, `allow_recording`, `broadcast_range`, `force_voice_chat`,
`login_timeout`; `spectator_interaction`, `spectator_player_possession`, `allow_pings`,
`use_natives` and `threaded_server_support` are read but not used (see "Not supported yet").

## Build

A horizon-mixins bundle (this folder) with one crate (`core`, std + `ring` + `log`, all in
Horizon's `Cargo.lock`), a trait-mod bridge (`bridge/`, the payload hook) and three injections:

```sh
horizon-mixins apply --workspace <horizon source> --mods <folder containing this bundle> --out <stage>
cd <stage> && cargo build --profile fast -p horizon-client --no-default-features --locked
```

`horizon_revision` in `horizon-mixin.toml` pins the Horizon commit it targets. Unit tests: `cargo test`
in `core/` (with Horizon's `Cargo.lock` copied next to it, to get the same crate versions).

The bridge takes `custom_payload_from_client` (a player's `voicechat:*` payload). The mixin's
`server_tick` injection (`flush.rs`) uses `native_server_players`, `native_player_channels`,
`native_send_custom_payload_to_player`, `native_tell` and `native_kick`; the last two outbound ones
and `custom_payload_from_client` / `player_channels` are pending API fragments in Horizon. The
commands use the hook points `register_commands` v1 and `perform_command` v1 (`register.rs`,
`perform.rs`). The engine glue has not been built against a Horizon checkout yet; the core crate's
unit tests run.

## Testing

`tools/vc_probe.py <game port> <screen session>` joins two or three bare protocol clients, connects
their voice, talks and prints a transcript with UUIDs replaced by names (handshake, voice
connection, keep-alives, distance cut-off, whisper, dimensions, disabled players, password groups,
leaving, quitting).

Untested: the real client mod with a graphical client, real Opus audio (the server never decodes
it), more than three players, Bedrock players, high packet rates.

## Not supported yet

- Volume categories and the category of a sound.
- Locational and entity audio channels and the audio player (a later Opus encoder).
- `spectator_interaction` and `spectator_player_possession`: spectators are routed like everyone
  else. Possession also needs the player a spectator watches, which Horizon does not expose.
- `allow_pings` (external pings), `use_natives`, `threaded_server_support`.
- Hidden groups and the two group flags of `voicechat:add_group` (always false on the wire);
  persistent groups exist server-side only (API).
- API events beyond the microphone packet, proxy support, translations.

## Licence and clean room

MIT, see [LICENSE](LICENSE).

HorizonVC is a clean-room implementation: it was written from public documentation and from the
behaviour of the client mod on the wire, with a test client that knows its own secret and so can
check a guess about the cipher or a packet layout. No code, text or assets of other voice chat
implementations are used.

## Contributing

Pull requests and improvements are always welcome! Direct pushes to main are disabled, every change goes through review.
