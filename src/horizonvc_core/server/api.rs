//! The crate's API for other native mods: connections, groups and their builder, audio senders,
//! static and locational audio channels, and volume categories. Nothing about players exists
//! before the voice server started (the first tick): calls then return `None`/`false`, except
//! category registration, which a mod does at server start, before the first tick.
use super::{frame, with_state, MicListener, PermissionCheck, State};
use crate::horizonvc_core::packets::{self as p, Group};
use std::net::SocketAddr;
use std::sync::{Mutex, MutexGuard};

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
        let mut id: [u8; 16] = crate::horizonvc_core::crypto::random();
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

/// A static audio channel (`createStaticAudioChannel`): one 20 ms Opus frame heard by one player only, like
/// group audio. The channel id goes where a player's sound carries the speaker's UUID (both UUIDs of the group
/// sound packet). `category`, when given, must be a registered volume category (see [`register_category`]);
/// it is checked but not yet written to the packet (the layout is not observed, see GAPS.md). False when the
/// listener has no voice connection, the category is unknown, or the server is not running.
pub fn send_static(
    channel: u128,
    to: u128,
    opus: &[u8],
    sequence: i64,
    category: Option<&str>,
) -> bool {
    if !category_known(&registry(), category) {
        return false;
    }
    with_state(|s| {
        let (addr, data) = s.static_frame(channel, to, opus, sequence)?;
        s.socket.as_ref()?.send_to(&data, addr).ok()
    })
    .flatten()
    .is_some()
}

/// A locational audio channel: one 20 ms Opus frame heard, with the distance attenuation of a player's sound,
/// by every connected player in `dimension` within `distance` blocks of `pos` (not the disabled ones). The
/// channel id takes the sender's UUID's place. Returns the number of datagrams sent. `category` is checked as
/// in [`send_static`].
pub fn send_locational(
    channel: u128,
    dimension: &str,
    pos: [f64; 3],
    distance: f64,
    opus: &[u8],
    sequence: i64,
    category: Option<&str>,
) -> usize {
    if !category_known(&registry(), category) {
        return 0;
    }
    with_state(|s| {
        let out = s.locational_frames(channel, dimension, pos, distance, opus, sequence);
        let Some(sock) = &s.socket else {
            return 0;
        };
        out.iter()
            .filter(|(to, data)| sock.send_to(data, *to).is_ok())
            .count()
    })
    .unwrap_or(0)
}

/// A volume category (`VolumeCategory`): the slider a client shows for the sounds in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub description: String,
    /// A 16 by 16 ARGB icon, row by row, or none.
    pub icon: Option<Vec<i32>>,
}

/// The number of pixels of a category icon.
pub const ICON_PIXELS: usize = 16 * 16;

/// The registered categories. Kept apart from the server state: a mod registers its categories at server
/// start, before the voice server exists.
#[derive(Debug, Default, PartialEq)]
pub struct Categories(Vec<Category>);

/// A category id: 1 to 64 characters, lowercase ASCII letters, digits and `_`.
pub fn valid_category_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl Categories {
    /// Adds a category, or replaces the one with the same id. False (and nothing changes) for an invalid id,
    /// an empty name or an icon that is not 16 by 16 pixels.
    pub fn register(
        &mut self,
        id: &str,
        name: &str,
        description: &str,
        icon: Option<&[i32]>,
    ) -> bool {
        if !valid_category_id(id)
            || name.trim().is_empty()
            || icon.is_some_and(|i| i.len() != ICON_PIXELS)
        {
            return false;
        }
        let category = Category {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            icon: icon.map(<[i32]>::to_vec),
        };
        match self.0.iter_mut().find(|c| c.id == id) {
            Some(existing) => *existing = category,
            None => self.0.push(category),
        }
        true
    }

    /// Removes a category; false when there was none with that id.
    pub fn unregister(&mut self, id: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|c| c.id != id);
        self.0.len() != before
    }

    pub fn get(&self, id: &str) -> Option<&Category> {
        self.0.iter().find(|c| c.id == id)
    }

    /// Every category, in registration order.
    pub fn all(&self) -> &[Category] {
        &self.0
    }
}

static CATEGORIES: Mutex<Categories> = Mutex::new(Categories(Vec::new()));

fn registry() -> MutexGuard<'static, Categories> {
    CATEGORIES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether a sound's category exists: no category always does.
fn category_known(reg: &Categories, category: Option<&str>) -> bool {
    category.is_none_or(|id| reg.get(id).is_some())
}

/// `registerVolumeCategory`: a slider for the sounds of the category `id` (see [`Categories::register`]).
pub fn register_category(id: &str, name: &str, description: &str, icon: Option<&[i32]>) -> bool {
    registry().register(id, name, description, icon)
}

/// `unregisterVolumeCategory`: false when no category has that id.
pub fn unregister_category(id: &str) -> bool {
    registry().unregister(id)
}

/// Every registered volume category.
pub fn categories() -> Vec<Category> {
    registry().all().to_vec()
}

impl State {
    /// The datagram carrying one static frame to `to`, if they have a voice connection.
    fn static_frame(
        &self,
        channel: u128,
        to: u128,
        opus: &[u8],
        sequence: i64,
    ) -> Option<(SocketAddr, Vec<u8>)> {
        let v = self.voicers.get(&to).filter(|v| v.connected)?;
        let addr = v.addr?;
        Some((addr, frame(&v.secret, &p::sound(channel, opus, sequence, None))))
    }

    /// The datagrams of one locational frame: a proximity sound to each connected, not disabled player in
    /// the dimension within `distance` of `pos`, in UUID order.
    fn locational_frames(
        &self,
        channel: u128,
        dimension: &str,
        pos: [f64; 3],
        distance: f64,
        opus: &[u8],
        sequence: i64,
    ) -> Vec<(SocketAddr, Vec<u8>)> {
        let plain = p::sound(channel, opus, sequence, Some((distance as f32, false)));
        let limit = distance * distance;
        let mut out: Vec<(u128, SocketAddr, Vec<u8>)> = self
            .voicers
            .iter()
            .filter_map(|(uuid, v)| {
                let addr = v.addr.filter(|_| v.connected && !v.disabled)?;
                let o = self.online.get(uuid)?;
                let d2: f64 = (0..3).map(|i| (o.pos[i] - pos[i]).powi(2)).sum();
                (o.dimension == dimension && d2 <= limit)
                    .then(|| (*uuid, addr, frame(&v.secret, &plain)))
            })
            .collect();
        out.sort_by_key(|(uuid, _, _)| *uuid);
        out.into_iter().map(|(_, addr, data)| (addr, data)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horizonvc_core::config::Config;
    use crate::horizonvc_core::crypto::open;
    use crate::horizonvc_core::wire::Reader;
    use crate::horizonvc_core::packets::id;
    use crate::horizonvc_core::server::{Online, Voicer};
    use std::time::Instant;

    const ICON: [i32; ICON_PIXELS] = [0x7F_FF_FF_FF; ICON_PIXELS];

    #[test]
    fn categories_register_replace_and_unregister() {
        let mut reg = Categories::default();
        assert!(reg.register("radio", "Radio", "Adjust the radio.", None));
        assert!(reg.register("winter_radio", "Winter Radio", "", Some(&ICON[..])));
        assert_eq!(reg.all().len(), 2);
        assert_eq!(reg.get("radio").unwrap().name, "Radio");
        // Same id: replaced, not added twice.
        assert!(reg.register("radio", "Radio 2", "Newer.", None));
        assert_eq!(reg.all().len(), 2);
        assert_eq!(reg.get("radio").unwrap().name, "Radio 2");
        assert_eq!(
            reg.get("winter_radio").unwrap().icon.as_ref().map(Vec::len),
            Some(ICON_PIXELS)
        );
        assert!(reg.unregister("radio"));
        assert!(!reg.unregister("radio"), "already gone");
        assert!(reg.get("radio").is_none());
    }

    #[test]
    fn invalid_categories_are_refused_and_change_nothing() {
        let mut reg = Categories::default();
        assert!(!reg.register("", "Empty", "", None));
        assert!(!reg.register("Radio", "Upper case", "", None));
        assert!(!reg.register("has space", "Space", "", None));
        assert!(!reg.register(&"a".repeat(65), "Long", "", None));
        assert!(!reg.register("radio", "  ", "", None), "a blank name");
        assert!(
            !reg.register("radio", "Radio", "", Some(&[0; 15][..])),
            "an icon of 15 pixels"
        );
        assert!(reg.all().is_empty());
        assert!(valid_category_id(&"a".repeat(64)));
        assert!(valid_category_id("winter_radio_2"));
    }

    #[test]
    fn a_sound_must_name_a_registered_category() {
        let mut reg = Categories::default();
        assert!(category_known(&reg, None), "no category is always fine");
        assert!(!category_known(&reg, Some("radio")));
        reg.register("radio", "Radio", "", None);
        assert!(category_known(&reg, Some("radio")));
    }

    /// A state with six players: 1 and 2 connected with addresses, 3 connected but disabled, 4 in the
    /// Nether, 5 connected without an address yet, 6 far away.
    fn world() -> State {
        let now = Instant::now();
        let mut s = State::new(Config::default(), 25666);
        let overworld = "minecraft:overworld";
        let players = [
            (1u128, overworld, [0.5, -60.0, 0.5], false),
            (2, overworld, [10.5, -60.0, 0.5], false),
            (3, overworld, [0.5, -60.0, 0.5], true),
            (4, "minecraft:the_nether", [0.5, -60.0, 0.5], false),
            (5, overworld, [0.5, -60.0, 0.5], false),
            (6, overworld, [200.5, -60.0, 0.5], false),
        ];
        for (uuid, dimension, pos, disabled) in players {
            s.online.insert(
                uuid,
                Online {
                    name: uuid.to_string(),
                    dimension: dimension.to_string(),
                    pos,
                    level: 0,
                    spectator: false,
                    has_mod: true,
                    joined: now,
                    kicked: false,
                },
            );
            let addr = (uuid != 5)
                .then(|| format!("127.0.0.1:{}", 40000 + uuid).parse::<SocketAddr>().unwrap());
            s.voicers.insert(
                uuid,
                Voicer {
                    secret: [uuid as u8; 16],
                    addr,
                    connected: true,
                    last_seen: now,
                    disabled,
                    group: None,
                    window: (now, 0),
                },
            );
        }
        s
    }

    /// Opens a server datagram the way a client does: 0xFF, then the sealed packet.
    fn open_for(secret: &[u8; 16], data: &[u8]) -> Vec<u8> {
        let mut r = Reader::new(data);
        assert_eq!(r.u8(), Ok(0xFF));
        let sealed = r.bytes(65_536).unwrap();
        open(secret, sealed).expect("authentic datagram")
    }

    #[test]
    fn a_static_frame_goes_to_one_connected_listener_as_group_sound() {
        let s = world();
        let channel = 0xABCD_u128;
        let (addr, data) = s.static_frame(channel, 1, &[1, 2, 3], 7).expect("connected");
        assert_eq!(addr, "127.0.0.1:40001".parse().unwrap());
        let plain = open_for(&[1; 16], &data);
        assert_eq!(plain[0], id::GROUP_SOUND);
        assert_eq!(&plain[1..17], &channel.to_be_bytes());
        assert_eq!(&plain[17..33], &channel.to_be_bytes());
        // Disabled but connected: the radio decides who hears it, so a frame is still built.
        assert!(s.static_frame(channel, 3, &[1], 7).is_some());
        assert!(s.static_frame(channel, 5, &[1], 7).is_none(), "no address yet");
        assert!(s.static_frame(channel, 99, &[1], 7).is_none(), "no voice connection");
    }

    #[test]
    fn a_locational_frame_reaches_connected_players_in_range_only() {
        let s = world();
        let channel = 0x5151_u128;
        let frames =
            s.locational_frames(channel, "minecraft:overworld", [0.5, -60.0, 0.5], 16.0, &[9, 9], 3);
        let addrs: Vec<SocketAddr> = frames.iter().map(|(a, _)| *a).collect();
        // 1 stands at the source, 2 is 10 blocks away; 3 is disabled, 4 is in the Nether, 5 has no
        // address, 6 is 200 blocks away.
        assert_eq!(
            addrs,
            vec![
                "127.0.0.1:40001".parse().unwrap(),
                "127.0.0.1:40002".parse().unwrap()
            ]
        );
        let plain = open_for(&[1; 16], &frames[0].1);
        assert_eq!(plain[0], id::PLAYER_SOUND);
        assert_eq!(&plain[1..17], &channel.to_be_bytes());
        assert_eq!(&plain[17..33], &channel.to_be_bytes());
        let mut r = Reader::new(&plain[33..]);
        assert_eq!(r.bytes(1024), Ok(&[9u8, 9][..]));
        assert_eq!(r.i64(), Ok(3));
        assert_eq!(r.i32().map(|b| f32::from_bits(b as u32)), Ok(16.0));
        assert_eq!(r.bool(), Ok(false));
    }

    #[test]
    fn a_locational_frame_with_nobody_in_range_sends_nothing() {
        let s = world();
        assert!(s
            .locational_frames(1, "minecraft:the_end", [0.0; 3], 50.0, &[1], 0)
            .is_empty());
        assert!(s
            .locational_frames(1, "minecraft:overworld", [500.0, 0.0, 0.0], 5.0, &[1], 0)
            .is_empty());
    }
}
