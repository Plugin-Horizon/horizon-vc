# HorizonVC

## Port status

Compared with Simple Voice Chat, server side (documentation only) (estimate): **81 %** of the original's commands, permissions, config keys, messages and features work today; **81 %** once our open Horizon PRs are merged; **81 %** with the engine changes we are writing; **81 %** with the newest Horizon SDK. Estimated count: ~30/~37 items. Status: not yet compiled after the 2026-11-20 round.

Clean-room reimplementation from public docs.

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
- Commands `/voicechat` (the documented name; aliases `/horizonvc`, `/hvc`): `help`, `test <player>` (operators), `invite <player>`
  (a clickable line that runs `/voicechat join <group id>`; the invitation stands in for the
  password once), `join <group name> [password]` (the name may contain spaces and matches case-insensitively; refused when more than one group has that name),
  `join <group id>`, `leave`.
- `force_voice_chat` with `login_timeout`: a player whose client has not asked for a voice secret
  within the timeout is kicked.
- Permissions `voicechat.listen`, `voicechat.speak`, `voicechat.groups`, `voicechat.admin`
  (the documented Simple Voice Chat names; default: everyone, `admin` for operators). They are declared as typed
  permission nodes, so a permissions mod such as LuckPerms answers them (the old `horizonvc.*` names are gone). A player without `listen` hears nothing, one
  without `speak` is not routed, one without `groups` cannot create or join groups.
- A Rust API for other mods (`horizonvc_core::api`): `connection_of`, `groups`, `create_group`
  (with persistent groups, which stay when empty), `remove_group`, `set_group`,
  `set_permission_check`, `add_mic_listener` (hear or drop each microphone packet),
  `send_as_player` (Opus data routed as a player's voice, for players without the mod) and
  `send_static` (a static audio channel: one listener, as group audio), `send_locational` (audio at a position) and volume categories (`register_category`, `unregister_category`, `categories`). Translations: `translations.properties` is read.

## Names that must stay `voicechat:`

The client mod only talks on plugin channels named `voicechat:<name>` (`request_secret`, `secret`,
`states`, `state`, `remove_state`, `add_group`, `remove_group`, `joined_group`, `create_group`,
`set_group`, `leave_group`, `update_state`), and the server recognises a client with the mod by those
channels. These ids, the packet layouts and the compatibility number are part of the protocol and
are not renamed. Everything else is HorizonVC's own: command, permission nodes, config, log prefix.

## Config

The original's `voicechat-server.properties` is read where Simple Voice Chat keeps it: `plugins/voicechat/` (Paper), then
`config/voicechat/` (mod loaders). Where neither exists, `config/horizonvc/horizonvc-server.properties` is written with
the defaults on the first start ([data/horizonvc/horizonvc-server.properties](data/horizonvc/horizonvc-server.properties)).
`translations.properties` is looked up in the same two folders. Keys:
`port` (-1: the game port), `bind_address` (empty: `server-ip`, `*`: every address),
`max_voice_distance`, `whisper_distance`, `codec`, `mtu_size`, `tcp_rate_limit`, `keep_alive`,
`enable_groups`, `voice_host`, `allow_recording`, `broadcast_range`, `force_voice_chat`,
`login_timeout`; `spectator_interaction`, `spectator_player_possession`, `allow_pings`,
`use_natives` and `threaded_server_support` are read but not used (see "Not supported yet").

## Build

A native Horizon mod: `horizon-mod.toml` plus this crate (`src/lib.rs` is the mod, `src/horizonvc_core/` the
protocol core). Unit tests: `cargo test`. It uses the mod API for raw `voicechat:*` payloads, the tick, commands and
typed permission nodes; it has not been run with a real client yet.

## Testing

`tools/vc_probe.py <game port> <screen session>` joins two or three bare protocol clients, connects
their voice, talks and prints a transcript with UUIDs replaced by names (handshake, voice
connection, keep-alives, distance cut-off, whisper, dimensions, disabled players, password groups,
leaving, quitting).

Untested: the real client mod with a graphical client, real Opus audio (the server never decodes
it), more than three players, Bedrock players, high packet rates.

## Not supported yet

- `spectator_player_possession`: needs the player a spectator watches, which Horizon does not expose (API gap, see the audit's GAPS_UPSTREAM).
- `allow_pings` (external pings): the public wiki says the server should answer them, but not how (the packet is not documented).
- `use_natives`: loads native libraries on dedicated servers; nothing to load on Horizon.
- `threaded_server_support`: read, unused (the default is `true` on Paper, Horizon has no Folia-style threads to support).
- Hidden groups and the two group flags of `voicechat:add_group` (always false on the wire): the public wiki does not describe them,
  so they are not guessed. Persistent groups exist server-side only (API).
- API events beyond the microphone packet, proxy support.

## Licence and clean room

MIT, see [LICENSE](LICENSE).

HorizonVC is a clean-room implementation: it was written from public documentation and from the
behaviour of the client mod on the wire, with a test client that knows its own secret and so can
check a guess about the cipher or a packet layout. No code, text or assets of other voice chat
implementations are used.

## Contributing

Pull requests and improvements are always welcome! Direct pushes to main are disabled, every change goes through review.
