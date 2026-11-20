# Clean room log

HorizonVC is an independent clean-room reimplementation of Simple Voice Chat (server side). Not affiliated.
Contains no code from the original. The original's source, jar and forks were never opened for this mod.

## Public documentation used

| Date | URL | Taken from it |
|---|---|---|
| 2026-11-20 | https://modrepo.de/minecraft/voicechat/wiki | Index of the wiki pages (page titles only). |
| 2026-11-20 | https://modrepo.de/minecraft/voicechat/wiki/server_config | Key names, default values and one-line descriptions of the server settings (`allow_pings`, `spectator_*`, `threaded_server_support`, `use_natives`, `enable_groups`), and the config file location for Bukkit and mod loaders. |
| 2026-11-20 | https://modrepo.de/minecraft/voicechat/wiki/commands | Command list (`help`, `test`, `invite`, `join <name>`, `join <id>`, `leave`), the `voicechat.admin` node on `test`, the rule that commands run in-game only. |
| 2026-11-20 | https://modrepo.de/minecraft/voicechat/wiki/permissions | Permission node names (`voicechat.listen`, `voicechat.speak`, `voicechat.groups`, `voicechat.admin`) and their defaults. |

Earlier rounds used the same pages (see the audit notes in the audit notes).

## Not in the public docs (so not implemented)

- Hidden groups and the two group flags of `voicechat:add_group`: the wiki does not describe them.
- The external ping packet behind `allow_pings`: the wiki says the server "should reply to external pings", but not how.
- Packet layouts and the cipher: taken from the behaviour of the client on the wire and checked with `tools/vc_probe.py`.

## Provenance note

`tests/fixtures/voicechat-server.properties` and `tests/fixtures/translations.properties` are copies of the
original plugin's files (the test names them as such). Their origin is to be confirmed by the owner before
release, because the clean-room rules forbid copying the original's assets.
