# Gaps

What HorizonVC cannot do yet, and what each gap needs.

## Horizon hooks and helpers

None missing: the mod uses the mod API (raw payload handlers, `send_payload`, `announced_channels`, `disconnect`, typed
permission nodes). The old `native_*` hooks are gone.

## Wording to review

Answer texts and colours are HorizonVC's own and open to taste:

- Command answers and help lines: `core/src/server/commands.rs`.
- Invitation line and its clickable run command: `commands.rs`.
- Status answer of `/horizonvc test`: `commands.rs`.
- Text for a player without the mod under `force_voice_chat`: `server.rs`.

## Command visibility

- `voicechat test` carries the `voicechat.admin` node (`Command::with_node`) and operator level 1 as fallback, so a
  permissions mod can make it visible to non-operators.
- `join` takes the first word as `<group:word>` and the rest as `[password:text]`; the core re-reads the line and
  tries the longest run of words that names a group, so names with spaces work, matched case-insensitively.

## Defaults

- `threaded_server_support` defaults to `true` (`config.rs`). The key is read but not used, so this changes nothing yet.

## Not supported yet

- `spectator_player_possession`: a spectator talking to the player they spectate needs the spectated entity (engine API).
- Hidden groups: the position of the hidden flag in `voicechat:add_group` is not in the public docs.
- `allow_pings`: the external ping packet is undocumented. `use_natives`, `threaded_server_support`: nothing to switch.
