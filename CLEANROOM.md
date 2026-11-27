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

## Not in the public docs

- The external ping packet behind `allow_pings`: the wiki says the server "should reply to external pings", but not how.
  Still not implemented (see the capture round below: nothing was observed).
- Packet layouts and the cipher: taken from the behaviour of the client on the wire and checked with `tools/vc_probe.py`
  and, since 2026-11-25, with captures of the real server and client (below).
- Hidden groups and the group flags of `voicechat:add_group`: the wiki does not describe them; observed 2026-11-25 (below).

## Provenance note

`tests/fixtures/voicechat-server.properties` and `tests/fixtures/translations.properties` are copies of the
original plugin's files (the test names them as such). Their origin is to be confirmed by the owner before
release, because the clean-room rules forbid copying the original's assets.

## Capture round 2026-11-25 (real client and real server as black boxes)

Rule kept: the released jars were only run, never opened, listed, decompiled, grepped or unpacked; nothing from
`_archive/unread_*` was touched; no production port or server folder was used (everything on 127.0.0.1, throwaway
ports 25620-25628 and 24460-24461). Downloads (Mojang's client files in `realclient/`, the jars below) live in
`prtmp/vcbench/` and are not in any repository.

What was run (all unmodified, from Modrinth / the PaperMC download API, SHA-1 or SHA-256 checked):

| What | Version | Role |
|---|---|---|
| Paper 26.3 build 262 | sha256 b2d24a0a... | throwaway server, port 25620, offline mode, compression off |
| voicechat-bukkit | 2.6.27 (sha1 1e367706...) | the original server, voice port 24460, `voice_host=127.0.0.1:24461` (a UDP tee in front) |
| Velocity | 4.3.1 | throwaway proxy 25622, modern forwarding, compression off |
| voicechat-velocity | 2.6.26 (sha1 ed73c2f1...; no 2.6.27 exists for Velocity) | the original proxy plugin, `voice_host=127.0.0.1:25623` |
| Fabric loader 0.20.1, Fabric API 0.167.0+26.3, voicechat-fabric | 2.6.27+26.3 (sha1 23442735...) | the real client, Minecraft 26.3 under Xvfb (`realclient/vcbench/fabric_client.py`) |

How it was recorded: `realclient/vcbench/vc_capture.py` is a tee for the game port (it logs every packet whose body is a
plugin message on a `voicechat:` channel; compression and encryption are off, so frames are plain) and for the voice UDP
port (every datagram). `tools/vc_decrypt.py` opens the datagrams with the session secret that the capture itself holds.
The client had no sound hardware: an ALSA "null" device (`realclient/vcbench/asound.conf`) lets its OpenAL open a
microphone; the menus were driven with `xdotool` and screenshots (voice chat menu, group menu, volume menu).
Server side, a small bench plugin (`realclient/vcbench/vcprobe/`) was written from the public API docs only
(modrepo.de wiki "API", Getting Started, Examples, and the public Javadocs of `voicechat.modrepo.de`): it calls
`groupBuilder()`, `registerVolumeCategory`, `setGroup`, the audio channel and audio player examples, and logs the
events. Compiling it needs the published API artifact (`voicechat-api-2.6.27.jar`, the Maven dependency the docs tell
every addon author to use); that jar was only handed to `javac` as a dependency, not read. This is the one place where
a released artifact of the original was used other than by running it; if the owner of this repository objects, delete
`realclient/vcbench/vcprobe/` and the captures that depend on it (hidden/persistent groups, categories, API sounds).

Observed (fixtures: `tests/fixtures/captures/*`, tests: `src/horizonvc_core/captured_tests.rs`):

1. **Hidden / persistent flag of `voicechat:add_group`** (`run1_tcp.jsonl`). Layout: group UUID, name (string), then three
   booleans password, persistent, hidden, then the type as i16 (0 normal, 1 open, 2 isolated). Created through the API with
   all combinations (normal/open/isolated, password, persistent, hidden): hidden groups ARE sent to the clients, with the
   third flag set (the group menu of the real client then does not list them; screenshot of the menu showed only the three
   visible groups). The second flag is "persistent" (true for API groups built persistent, false for groups made in the menu).
   A non-persistent API group is not announced and not in `getGroups()` until a first player is added; the same for a
   hidden non-persistent one. On the join, the server sends: `states`, every `add_category`, every `add_group`, `secret`.
   The client's own `create_group` payload (menu, all three types, with and without password) is `name, password flag
   [+ string], type i16`; `set_group` is `uuid, password flag [+ string]`; `leave_group` is empty. A wrong password is
   answered `joined_group 00 01`; leaving by the menu: `state`, `joined_group 00 00`, `remove_group`.
2. **Volume categories** (`run1_tcp.jsonl`, `sounds_plain.txt`): `add_category` = id, name, description (flag + string),
   icon (flag + 256 ints when present); between them two flag bytes that were 0 in every capture (their meaning is unknown, written as 0; the first capture-based test found the old layout lacked them); `remove_category` = id. The real client listed a "Radio" slider in its volume menu
   after exactly these bytes. Static (id 3) and locational (id 4) channel sounds carry the category as flag bit 0b10 and the
   string after the flags byte. Locational channels use their own packet id 4: channel id twice, source position (three
   f64), audio, sequence, distance (f32), flags; HorizonVC had used the player-sound packet (id 2) for them, now fixed.
3. **Proxy** (`proxy1_tcp.jsonl`, `backend_side_tcp.jsonl`): the Velocity plugin forwards every `voicechat:*` message
   unchanged except the `secret`: it keeps the key and the player but replaces the port with its own UDP port and the voice
   host with its own `voice_host`. So a backend behind it only needs the ordinary secret packet with its real voice port in it
   (the proxy reads it from there). The client's UDP then goes to the proxy; the real Paper plugin authenticated the client
   through the proxy (log line "Successfully authenticated player"). Not run: a Horizon backend behind the proxy (no Horizon
   engine run in this round); the proxy-to-backend UDP hop was not recorded.
4. **Events** (docs-driven; the bench plugin logged them): `PlayerConnected`, `PlayerDisconnected`, `PlayerStateChanged`,
   `CreateGroup`, `JoinGroup`, `LeaveGroup`, `RemoveGroup`, `Register/UnregisterVolumeCategory`, `VoiceHost`. Observed order:
   `player_state` (disconnected) at the secret request, `voice_host` when the secret is built, `player_state` per state change,
   `player_connected` last. An API-built persistent group raises `create_group` with no player; `setGroup` raises `join_group`
   then `player_state`; removing raises `remove_group`. `VoiceHostEvent`, the sound-packet events, `VoiceDistanceEvent` and
   the server started/stopped events were not implemented (see GAPS.md); no event is cancellable here.
5. **allow_pings**: the shipped config has `allow_pings=true`. During roughly five minutes of real client sessions the only
   UDP traffic was the keep-alive pair (client and server, once a second) and the authentication; the real client sent no
   ping. 1024 short unauthenticated datagrams (every first byte 0-255 in four lengths, `realclient/vcbench/udp_fuzz.py`)
   got no reply. The external ping format could not be observed, so nothing was guessed or implemented.

One slip, reported: the very first start of the bench Paper (before its config was edited) bound the plugin's default
voice port 24454 for about 30 seconds on 127.0.0.1; nothing else was listening there, and it was stopped before the real run.
