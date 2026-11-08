# Gaps

What HorizonVC cannot do yet, and what each gap needs.

## Horizon hooks and helpers

`flush.rs` sends the `voicechat:*` payloads with `native_send_custom_payload_to_player` (any non-`horizon:` channel, up
to 1 MiB), kicks with `native_kick`, and reads which clients have the mod from `native_player_channels`; `bridge/` hears
`custom_payload_from_client`. All four are pending API fragments (`send_custom_payload_to_player`, `kick`,
`custom_payload_from_client`, `player_channels`): they exist in the code but have no stamped API number yet. The engine
glue has not been built.

## Wording to review

Answer texts and colours are HorizonVC's own and open to taste:

- Command answers and help lines: `core/src/server/commands.rs`.
- Invitation line and its clickable run command: `commands.rs`.
- Status answer of `/horizonvc test`: `commands.rs`.
- Text for a player without the mod under `force_voice_chat`: `server.rs`.

## Command visibility

- `horizonvc test` is registered at operator level 1 (`commands.rs` USAGES, `register.rs`), matching the
  `horizonvc.admin` default. Non-operators do not see it in their tree and get Horizon's unknown-command answer. A
  permissions mod granting `horizonvc.admin` to a non-operator cannot make it visible, because the usage has no
  permission node.
- Group names are matched case-sensitively in `join`, while players are matched case-insensitively.
- Group names containing a space cannot be typed, since `<group:word>` takes one word.

## Defaults

- `threaded_server_support` defaults to `true` (`config.rs`). The key is read but not used, so this changes nothing yet.

## Not supported yet

See the README: spectator routing and possession, locational and entity audio, volume categories, `allow_pings`, hidden
groups and API events beyond the microphone packet.
