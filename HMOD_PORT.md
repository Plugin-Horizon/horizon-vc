Parity: before ~75% -> after ~81% (~30/~37 items; method: audit + PARITY.md estimate; items = commands, nodes, config keys, features; clean room, docs only)

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
