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

/// Every group the voice server knows: id, name, has a password, type (0 normal, 1 open, 2 isolated),
/// persistent, hidden. (A non-persistent group the API built is not among them until its first player joins.)
pub fn groups() -> Vec<(u128, String, bool, i16, bool, bool)> {
    with_state(|s| {
        s.groups
            .iter()
            .map(|g| (g.id, g.name.clone(), g.password.is_some(), g.kind, g.persistent, g.hidden))
            .collect()
    })
    .unwrap_or_default()
}

/// `groupBuilder()...build()`: a new group. A persistent group is announced to every client at once and stays when
/// nobody is in it. Any other group comes to be, and is announced, when its first player is added (`set_group`),
/// and disappears once its last member leaves (a real server behaves so: its `getGroups()` does not list it before).
/// A hidden group is announced but left out of the clients' group menu.
pub fn create_group(
    name: &str,
    password: Option<&str>,
    kind: i16,
    persistent: bool,
    hidden: bool,
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
            persistent,
            hidden,
        };
        let id = g.id;
        if persistent {
            s.events.push(event_group("create_group", &g, None));
            s.broadcast("voicechat:add_group", p::add_group(&g));
            s.groups.push(g);
        } else {
            s.pending.push(g);
        }
        id
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
        if let Some(g) = s.groups.iter_mut().find(|g| g.id == id) {
            g.persistent = false;
        }
        for m in members {
            s.leave(m);
        }
        s.pending.retain(|g| g.id != id);
        if let Some(i) = s.groups.iter().position(|g| g.id == id) {
            let g = s.groups.remove(i);
            s.events.push(event_group("remove_group", &g, None));
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

/// Lets a permissions mod answer `voicechat.listen`, `.speak`, `.groups` and `.admin`.
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
/// it is written into the sound packet (`AudioChannel.setCategory`), so the client's slider for it scales the frame. False when the
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
        let (addr, data) = s.static_frame(channel, to, opus, sequence, category)?;
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
        let out = s.locational_frames(channel, dimension, pos, distance, opus, sequence, category);
        let Some(sock) = &s.socket else {
            return 0;
        };
        out.iter()
            .filter(|(to, data)| sock.send_to(data, *to).is_ok())
            .count()
    })
    .unwrap_or(0)
}

// ---- Events (the public API's events, as lines for the mods that subscribed) ----

/// A UUID as dashed text.
pub fn uuid_text(id: u128) -> String {
    let h = format!("{id:032x}");
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// A UUID with or without dashes.
pub fn parse_uuid(s: &str) -> Option<u128> {
    let plain: String = s.chars().filter(|c| *c != '-').collect();
    (plain.len() == 32).then(|| u128::from_str_radix(&plain, 16).ok()).flatten()
}

fn type_name(kind: i16) -> &'static str {
    match kind {
        p::OPEN => "OPEN",
        p::ISOLATED => "ISOLATED",
        _ => "NORMAL",
    }
}

fn parse_type(name: &str) -> Option<i16> {
    match name.to_ascii_uppercase().as_str() {
        "NORMAL" => Some(p::NORMAL),
        "OPEN" => Some(p::OPEN),
        "ISOLATED" => Some(p::ISOLATED),
        _ => None,
    }
}

/// `PlayerStateChangedEvent`: player, disabled, disconnected.
pub fn event_player_state(player: u128, disabled: bool, disconnected: bool) -> String {
    format!("player_state\t{}\t{disabled}\t{disconnected}", uuid_text(player))
}

/// `PlayerConnectedEvent` (`player_connected`) and `PlayerDisconnectedEvent` (`player_disconnected`): player.
pub fn event_player(kind: &str, player: u128) -> String {
    format!("{kind}\t{}", uuid_text(player))
}

/// `CreateGroupEvent` (`create_group`), `JoinGroupEvent` (`join_group`), `LeaveGroupEvent` (`leave_group`) and
/// `RemoveGroupEvent` (`remove_group`): group id, name, has a password, type name, persistent, hidden, then the
/// player (empty when no player made it: the API created or removed the group).
pub fn event_group(kind: &str, g: &Group, player: Option<u128>) -> String {
    format!(
        "{kind}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        uuid_text(g.id),
        g.name,
        g.password.is_some(),
        type_name(g.kind),
        g.persistent,
        g.hidden,
        player.map(uuid_text).unwrap_or_default()
    )
}

/// The events since the last call, oldest first.
pub fn take_events() -> Vec<String> {
    with_state(|s| std::mem::take(&mut s.events)).unwrap_or_default()
}

/// The mods that get the events (see `handle`, `subscribe`).
pub fn subscribers() -> Vec<String> {
    with_state(|s| s.subscribers.clone()).unwrap_or_default()
}

pub fn subscribe(id: &str) -> bool {
    with_state(|s| {
        if !s.subscribers.iter().any(|m| m == id) {
            s.subscribers.push(id.to_owned());
        }
    })
    .is_some()
}

pub fn unsubscribe(id: &str) {
    with_state(|s| s.subscribers.retain(|m| m != id));
}

/// Answers one call of the mod service `voicechat:api`: `<op>` then its arguments, tab-separated; one text line back
/// (`true`/`false`, a UUID, or `none`); None for an operation or arguments it does not know.
///
/// Operations: `version`, `subscribe <mod id>`, `unsubscribe <mod id>`, `groups` (one line per group, as the group
/// events carry the group, newline-separated), `create_group <name> <NORMAL|OPEN|ISOLATED> <persistent> <hidden>
/// [password]`, `remove_group <uuid>`, `set_group <player uuid> <group uuid|none>`, `register_category <id> <name>
/// <description>`, `unregister_category <id>`. A subscribed mod's own service `<mod id>:voicechat_event` is called
/// with one event line per event: `player_state`, `player_connected`, `player_disconnected`, `create_group`,
/// `join_group`, `leave_group`, `remove_group`, `category_registered`, `category_unregistered`.
pub fn handle(payload: &str) -> Option<String> {
    let mut parts = payload.split('\t');
    let op = parts.next()?;
    let a: Vec<&str> = parts.collect();
    let word = |i: usize| a.get(i).copied();
    let flag = |v: bool| v.to_string();
    let truth = |i: usize| word(i).map(|w| w == "true");
    Some(match op {
        "version" => "0".into(),
        "subscribe" => flag(subscribe(word(0)?)),
        "unsubscribe" => {
            unsubscribe(word(0)?);
            "true".into()
        }
        "groups" => groups()
            .into_iter()
            .map(|(id, name, pw, kind, persistent, hidden)| {
                format!("{}\t{name}\t{pw}\t{}\t{persistent}\t{hidden}", uuid_text(id), type_name(kind))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "create_group" => match create_group(word(0)?, word(4), parse_type(word(1)?)?, truth(2)?, truth(3)?) {
            Some(id) => uuid_text(id),
            None => "none".into(),
        },
        "remove_group" => flag(remove_group(parse_uuid(word(0)?)?)),
        "set_group" => {
            let group = match word(1)? {
                "none" => None,
                id => Some(parse_uuid(id)?),
            };
            flag(set_group(parse_uuid(word(0)?)?, group))
        }
        "register_category" => flag(register_category(word(0)?, word(1)?, word(2)?, None)),
        "unregister_category" => flag(unregister_category(word(0)?)),
        _ => return None,
    })
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
    let ok = registry().register(id, name, description, icon);
    if ok {
        // Players already connected get the slider at once (`VoicechatServerApi.registerVolumeCategory`).
        with_state(|s| {
            s.events.push(format!("category_registered\t{id}"));
            s.broadcast("voicechat:add_category", p::add_category(id, name, description, icon))
        });
    }
    ok
}

/// `unregisterVolumeCategory`: false when no category has that id.
pub fn unregister_category(id: &str) -> bool {
    let ok = registry().unregister(id);
    if ok {
        with_state(|s| {
            s.events.push(format!("category_unregistered\t{id}"));
            s.broadcast("voicechat:remove_category", p::remove_category(id))
        });
    }
    ok
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
        category: Option<&str>,
    ) -> Option<(SocketAddr, Vec<u8>)> {
        let v = self.voicers.get(&to).filter(|v| v.connected)?;
        let addr = v.addr?;
        Some((addr, frame(&v.secret, &p::sound_in(channel, opus, sequence, None, category))))
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
        category: Option<&str>,
    ) -> Vec<(SocketAddr, Vec<u8>)> {
        // The sound of a locational channel carries its source position (a real server's packet, id 4).
        let plain = p::locational_sound(channel, pos, opus, sequence, distance as f32, category);
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
        let (addr, data) = s.static_frame(channel, 1, &[1, 2, 3], 7, None).expect("connected");
        assert_eq!(addr, "127.0.0.1:40001".parse().unwrap());
        let plain = open_for(&[1; 16], &data);
        assert_eq!(plain[0], id::GROUP_SOUND);
        assert_eq!(&plain[1..17], &channel.to_be_bytes());
        assert_eq!(&plain[17..33], &channel.to_be_bytes());
        // Disabled but connected: the radio decides who hears it, so a frame is still built.
        assert!(s.static_frame(channel, 3, &[1], 7, None).is_some());
        assert!(s.static_frame(channel, 5, &[1], 7, None).is_none(), "no address yet");
        assert!(s.static_frame(channel, 99, &[1], 7, None).is_none(), "no voice connection");
    }

    #[test]
    fn a_locational_frame_reaches_connected_players_in_range_only() {
        let s = world();
        let channel = 0x5151_u128;
        let frames =
            s.locational_frames(channel, "minecraft:overworld", [0.5, -60.0, 0.5], 16.0, &[9, 9], 3, None);
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
        // The packet of a locational channel (captured from a real server): id 4, the channel id twice, the source
        // position, the audio, the sequence, the distance, the flags.
        assert_eq!(plain[0], id::LOCATIONAL_SOUND);
        assert_eq!(&plain[1..17], &channel.to_be_bytes());
        assert_eq!(&plain[17..33], &channel.to_be_bytes());
        let mut r = Reader::new(&plain[33..]);
        for want in [0.5f64, -60.0, 0.5] {
            let bits = (r.i64().unwrap()) as u64;
            assert_eq!(f64::from_bits(bits), want);
        }
        assert_eq!(r.bytes(1024), Ok(&[9u8, 9][..]));
        assert_eq!(r.i64(), Ok(3));
        assert_eq!(r.i32().map(|b| f32::from_bits(b as u32)), Ok(16.0));
        assert_eq!(r.bool(), Ok(false));
    }

    #[test]
    fn a_locational_frame_with_nobody_in_range_sends_nothing() {
        let s = world();
        assert!(s
            .locational_frames(1, "minecraft:the_end", [0.0; 3], 50.0, &[1], 0, None)
            .is_empty());
        assert!(s
            .locational_frames(1, "minecraft:overworld", [500.0, 0.0, 0.0], 5.0, &[1], 0, None)
            .is_empty());
    }

    // Simple Voice Chat `AudioChannel.setCategory("radio")`: the static frame names the category after its flags
    // byte, so the client scales it by the "radio" slider.
    #[test]
    fn a_static_frame_names_its_volume_category() {
        let s = world();
        let (_, data) = s.static_frame(0xABCD, 1, &[1, 2, 3], 7, Some("radio")).expect("connected");
        let plain = open_for(&[1; 16], &data);
        let mut r = Reader::new(&plain[33..]);
        assert_eq!(r.bytes(1024), Ok(&[1u8, 2, 3][..]));
        assert_eq!(r.i64(), Ok(7));
        assert_eq!(r.u8(), Ok(0b10));
        assert_eq!(r.string(16), Ok("radio".to_string()));
    }

    #[test]
    fn a_locational_frame_names_its_volume_category() {
        let s = world();
        let frames = s.locational_frames(0x5151, "minecraft:overworld", [0.5, -60.0, 0.5], 16.0, &[9], 3, Some("radio"));
        let plain = open_for(&[1; 16], &frames[0].1);
        let mut r = Reader::new(&plain[33 + 24..]);
        assert_eq!(r.bytes(1024), Ok(&[9u8][..]));
        assert_eq!(r.i64(), Ok(3));
        assert_eq!(r.i32().map(|b| f32::from_bits(b as u32)), Ok(16.0));
        assert_eq!(r.u8(), Ok(0b10));
        assert_eq!(r.string(16), Ok("radio".to_string()));
    }

    // An unregistered category is refused (nothing is sent), a registered one is accepted.
    #[test]
    fn only_registered_categories_may_be_named() {
        assert!(register_category("radio_test_cat", "Radio", "Adjust the lobby radio volume.", None));
        assert!(category_known(&registry(), Some("radio_test_cat")));
        assert!(!category_known(&registry(), Some("never_registered")));
        assert!(unregister_category("radio_test_cat"));
    }

    #[test]
    fn the_service_builds_groups_and_names_events_in_order() {
        assert_eq!(handle("version").as_deref(), Some("0"));
        assert_eq!(handle("nonsense"), None);
        assert_eq!(handle("create_group\tx\tSILLY\ttrue\tfalse"), None, "an unknown type");
        let g = Group { id: 0xab, name: "n".into(), password: Some("p".into()), kind: p::ISOLATED, persistent: true, hidden: true };
        assert_eq!(
            event_group("create_group", &g, None),
            "create_group\t00000000-0000-0000-0000-0000000000ab\tn\ttrue\tISOLATED\ttrue\ttrue\t"
        );
        assert_eq!(event_player_state(1, true, false), "player_state\t00000000-0000-0000-0000-000000000001\ttrue\tfalse");
        assert_eq!(parse_uuid("00000000-0000-0000-0000-0000000000ab"), Some(0xab));
        assert_eq!(parse_uuid("zz"), None);
    }
}
