//! The crate's API for other native mods, for other native mods (connections, groups and their builder, audio senders, static audio channels). Nothing here exists before the
//! voice server started (the first tick): calls then return `None`/`false`.
use super::{frame, with_state, MicListener, PermissionCheck};
use crate::packets::{self as p, Group};

/// What the API tells of a player with the mod (`VoicechatConnection`).
#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub installed: bool,
    pub connected: bool,
    pub disabled: bool,
    pub group: Option<u128>,
}

/// `getConnectionOf`: `None` for a player who is not on or has no voice chat mod.
pub fn connection_of(player: u128) -> Option<Connection> {
    with_state(|s| {
        let v = s.voicers.get(&player)?;
        Some(Connection {
            installed: true,
            connected: v.connected,
            disabled: v.disabled,
            group: v.group,
        })
    })
    .flatten()
}

/// Every group: id, name, has a password, type (0 normal, 1 open, 2 isolated), persistent.
pub fn groups() -> Vec<(u128, String, bool, i16, bool)> {
    with_state(|s| {
        s.groups
            .iter()
            .map(|g| {
                (
                    g.id,
                    g.name.clone(),
                    g.password.is_some(),
                    g.kind,
                    s.persistent.contains(&g.id),
                )
            })
            .collect()
    })
    .unwrap_or_default()
}

/// `groupBuilder()...build()`: a new group, announced to every client. A persistent group stays when nobody is
/// in it; another one disappears once its last member leaves (and is announced when its first player is added, which `set_group` does).
pub fn create_group(
    name: &str,
    password: Option<&str>,
    kind: i16,
    persistent: bool,
) -> Option<u128> {
    if !(p::NORMAL..=p::ISOLATED).contains(&kind) || name.trim().is_empty() {
        return None;
    }
    with_state(|s| {
        let mut id: [u8; 16] = crate::crypto::random();
        id[6] = (id[6] & 0x0F) | 0x40;
        id[8] = (id[8] & 0x3F) | 0x80;
        let g = Group {
            id: u128::from_be_bytes(id),
            name: name.into(),
            password: password.map(str::to_string),
            kind,
        };
        if persistent {
            s.persistent.insert(g.id);
        }
        s.broadcast("voicechat:add_group", p::add_group(&g));
        s.groups.push(g.clone());
        g.id
    })
}

/// `removeGroup`: its members leave it first.
pub fn remove_group(id: u128) -> bool {
    with_state(|s| {
        let members: Vec<u128> = s
            .voicers
            .iter()
            .filter(|(_, v)| v.group == Some(id))
            .map(|(u, _)| *u)
            .collect();
        s.persistent.remove(&id);
        for m in members {
            s.leave(m);
        }
        if let Some(i) = s.groups.iter().position(|g| g.id == id) {
            s.groups.remove(i);
            s.broadcast("voicechat:remove_group", p::remove_group(id));
        }
        true
    })
    .unwrap_or(false)
}

/// `VoicechatConnection.setGroup`: puts a player with the mod into a group (no password asked) or out of theirs.
pub fn set_group(player: u128, group: Option<u128>) -> bool {
    with_state(|s| match group {
        Some(id) => {
            s.invites.insert((player, id));
            s.join(player, id, None).is_ok()
        }
        None => {
            s.leave(player);
            true
        }
    })
    .unwrap_or(false)
}

/// Lets a permissions mod answer `horizonvc.listen`, `.speak`, `.groups` and `.admin`.
pub fn set_permission_check(check: PermissionCheck) {
    with_state(|s| s.permissions = check);
}

/// Hears every microphone packet before it is routed (and may drop it).
pub fn add_mic_listener(listener: MicListener) -> bool {
    with_state(|s| s.mic_listeners.push(listener)).is_some()
}

/// An audio sender (`createAudioSender`): Opus data sent as if `player` spoke it, routed as their microphone is
/// (proximity, whisper, their group). For players without the mod; the caller paces the packets.
pub fn send_as_player(player: u128, opus: &[u8], sequence: i64, whispering: bool) -> usize {
    with_state(|s| {
        let out = s.route(player, opus, sequence, whispering);
        let n = out.len();
        if let Some(sock) = &s.socket {
            for (to, data) in out {
                let _ = sock.send_to(&data, to);
            }
        }
        n
    })
    .unwrap_or(0)
}

/// A static audio channel (`createStaticAudioChannel`): Opus data heard by one player only, like group audio.
/// The channel id goes where a player's sound carries the speaker's UUID (both UUIDs of the group sound packet).
pub fn send_static(channel: u128, to: u128, opus: &[u8], sequence: i64) -> bool {
    with_state(|s| {
        let v = s.voicers.get(&to).filter(|v| v.connected)?;
        let addr = v.addr?;
        let data = frame(&v.secret, &p::sound(channel, opus, sequence, None));
        s.socket.as_ref()?.send_to(&data, addr).ok()
    })
    .flatten()
    .is_some()
}
