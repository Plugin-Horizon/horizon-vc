Parity: ~81% now / ~81% with our PRs (~30/~37 items; method: audit + PARITY.md estimate; items = commands, nodes, config keys, features; clean room, docs only)

## Checklist round 100 (blind, not compiled; docs only, 2026-11-20)
Walked the public wiki pages (Commands, Server Config, Permissions; see CLEANROOM.md). Ticked:
- [x] Commands `help`, `test`, `invite`, `join <name>`, `join <id>`, `leave`: names, args, `voicechat.admin` on `test`.
- [x] Command root `/voicechat` as documented; `/horizonvc`, `/hvc` are extra aliases (kept).
- [x] Console: the docs say commands run in-game only; the port answers them only for a player with the mod.
- [x] Permission nodes `voicechat.listen|speak|groups|admin` (defaults as the docs).
- [x] Server config keys: all 19 documented keys are read, with their defaults (`use_natives` was missing from the key list; added).
- [x] Config file and translations location (`plugins/voicechat/`, `config/voicechat/`).
- [ ] `spectator_player_possession`: API-blocked (spectated entity, GAPS_UPSTREAM).
- [ ] `allow_pings`: the external ping packet is not in the public docs.
- [ ] Hidden groups and the add_group flags: not in the public docs.
- [~] `use_natives`, `threaded_server_support`: read, no effect on Horizon (documented as such).
- Not-done list: Bedrock, proxy support, more API events (outside the 19 keys, see README).

## Round 3 (blind, not compiled)
- Closed: the config file is found where the original keeps it on either platform (`plugins/voicechat/`, then `config/voicechat/`,
  then the port's own fallback); `translations.properties` likewise (`config.rs` `original_path`, `server.rs` `LOADER_TRANSLATIONS_PATH`).
- Already done by earlier rounds (the audit text was out of date): `voicechat.*` nodes, group names with spaces, locational/entity
  audio and categories (`server/api.rs`), the translations file, spectator silencing (`spectator_interaction`).
- Open: `spectator_player_possession` (needs the spectated entity, see GAPS_UPSTREAM), hidden groups (the wire flag order of
  `add_group` is not in the public docs, so it is not guessed), `allow_pings` (external ping packet is undocumented), `use_natives`
  and `threaded_server_support` (nothing to switch on Horizon).


## Audit fixes
- Permission nodes renamed to the documented `voicechat.listen|speak|groups|admin` (the `horizonvc.*` names are not kept as aliases: the three default-true nodes would make an alias OR-check grant what a `voicechat.*` denial refuses).
- `voicechat test` usages carry the `voicechat.admin` node.
- `join` accepts group names with spaces and case-insensitive names.
- Gaps: spectator interaction/possession, categories, locational/entity audio, hidden groups, translations, `allow_pings` unchanged.

## Round 100c (volume categories on the wire)
- `packets::sound_in`: the closing flags byte of a sound packet gets bit `0b10` and the category id follows as a string; `send_static` / `send_locational` pass the category (before: checked only). `add_category` / `remove_category` packets (`voicechat:add_category`, `voicechat:remove_category`, added to the clientbound channels) are sent to every player on `request_secret` and broadcast on register / unregister.
- Tests: `sounds_carry_their_volume_category`, `category_packets`, `a_static_frame_names_its_volume_category`, `a_locational_frame_names_its_volume_category`, `only_registered_categories_may_be_named`.
- Unverified (clean room: no jar opened): the flag bit, the string format and the field order of `add_category` (description and icon each behind a flag byte). Check with a real client before a deploy; DECISIONS has the entry.
