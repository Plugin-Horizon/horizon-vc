//! The voice server: state shared by the game thread (plugin channel messages, the players'
//! positions each tick) and the UDP thread (authentication, keep-alives, audio routing).
use crate::config::{server_property, Config};
use crate::crypto::{open, random, seal};
use crate::packets::{self as p, id, FromClient, Group, PlayerState, Voice};
use crate::routing::{recipients, Delivery, Listener};
use crate::wire::{Reader, Writer};
use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, UdpSocket};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub mod api;
mod commands;

pub use commands::USAGES;

/// Where the config is read from, relative to the server's directory.
pub const CONFIG_PATH: &str = "config/horizonvc/horizonvc-server.properties";

/// Chat line for a client that announces a different protocol compatibility version.
const INCOMPATIBLE: &str = "Voice chat is unavailable: your client mod speaks a different protocol version than this server.\nInstall the 2.6.x release of the voice chat client mod to use it.";

/// The text a player without the mod gets with `force_voice_chat` (shown as the kick reason).
pub const MOD_REQUIRED: &str =
    "You need Simple Voice Chat (version 2.6.x) installed to play on this server.";

/// A player on the server, as the game thread last saw them.
#[derive(Clone, Debug)]
pub struct Guest<'a> {
    pub uuid: u128,
    pub name: &'a str,
    pub dimension: &'a str,
    pub pos: [f64; 3],
    /// The operator level, 0 to 4.
    pub level: u8,
    /// Whether the client registered a `voicechat:` plugin channel, i.e. has the voice chat mod
    /// (`native_player_channels`).
    pub has_mod: bool,
}

/// Asks whether a player has a permission node (`horizonvc.listen`, `horizonvc.speak`, `horizonvc.groups`,
/// `horizonvc.admin`), given their operator level. Another mod (a permissions mod) can set its own.
pub type PermissionCheck = fn(player: u128, level: u8, node: &str) -> bool;

/// Defaults: everyone listens, speaks and uses groups; `horizonvc.admin` is for operators.
pub fn default_permissions(_player: u128, level: u8, node: &str) -> bool {
    node != "horizonvc.admin" || level >= 1
}

/// Hears a player's microphone packet before it is routed: the speaker, the Opus data, whether they whisper.
/// Returning true drops the packet. Runs on the voice thread with the state locked: it must not call this
/// crate's API.
pub type MicListener = Box<dyn Fn(u128, &[u8], bool) -> bool + Send>;

/// Something for the game thread to send to a player.
#[derive(Debug, PartialEq)]
pub enum Out {
    Payload {
        player: u128,
        channel: &'static str,
        data: Vec<u8>,
    },
    Chat {
        player: u128,
        text: String,
    },
    /// A chat line that runs `command` when clicked.
    ChatCommand {
        player: u128,
        text: String,
        command: String,
    },
    Kick {
        player: u128,
        text: String,
    },
}

struct Online {
    name: String,
    dimension: String,
    pos: [f64; 3],
    level: u8,
    has_mod: bool,
    joined: Instant,
    kicked: bool,
}

/// A player whose client asked for a secret (has the mod).
struct Voicer {
    secret: [u8; 16],
    addr: Option<SocketAddr>,
    connected: bool,
    last_seen: Instant,
    disabled: bool,
    group: Option<u128>,
    window: (Instant, i32),
}

pub struct State {
    cfg: Config,
    port: i32,
    online: HashMap<u128, Online>,
    voicers: HashMap<u128, Voicer>,
    groups: Vec<Group>,
    outbox: Vec<Out>,
    socket: Option<UdpSocket>,
    /// Groups kept when empty (made through the API).
    persistent: HashSet<u128>,
    /// (player, group): invitations, which join without the password.
    invites: HashSet<(u128, u128)>,
    permissions: PermissionCheck,
    mic_listeners: Vec<MicListener>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn lock() -> MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Writes to the game's log (`logs/latest.log`) through the `log` facade, which Horizon's logger
/// takes; a line on stderr alone would not reach the server log.
fn log(line: &str) {
    log::info!("[HorizonVC] {line}");
}

fn log_warn(line: &str) {
    log::warn!("[HorizonVC] {line}");
}

impl State {
    pub fn new(cfg: Config, port: i32) -> Self {
        Self {
            cfg,
            port,
            online: HashMap::new(),
            voicers: HashMap::new(),
            groups: Vec::new(),
            outbox: Vec::new(),
            socket: None,
            persistent: HashSet::new(),
            invites: HashSet::new(),
            permissions: default_permissions,
            mic_listeners: Vec::new(),
        }
    }

    fn may(&self, player: u128, node: &str) -> bool {
        let level = self.online.get(&player).map_or(0, |o| o.level);
        (self.permissions)(player, level, node)
    }

    fn state_of(&self, uuid: u128) -> PlayerState {
        let v = self.voicers.get(&uuid);
        PlayerState {
            disabled: v.is_some_and(|v| v.disabled),
            disconnected: !v.is_some_and(|v| v.connected),
            uuid,
            name: self
                .online
                .get(&uuid)
                .map(|o| o.name.clone())
                .unwrap_or_default(),
            group: v.and_then(|v| v.group),
        }
    }

    fn send(&mut self, player: u128, channel: &'static str, data: Vec<u8>) {
        self.outbox.push(Out::Payload {
            player,
            channel,
            data,
        });
    }

    /// To every online player with the mod.
    fn broadcast(&mut self, channel: &'static str, data: Vec<u8>) {
        let mut to: Vec<u128> = self
            .voicers
            .keys()
            .filter(|u| self.online.contains_key(*u))
            .copied()
            .collect();
        to.sort();
        for player in to {
            self.send(player, channel, data.clone());
        }
    }

    fn broadcast_state(&mut self, uuid: u128) {
        let s = p::state(&self.state_of(uuid));
        self.broadcast("voicechat:state", s);
    }

    /// Groups nobody is in any more are gone (after leaving a group and leaving the server;
    /// switching groups alone leaves the old one standing).
    fn remove_empty_groups(&mut self) {
        let (keep, gone): (Vec<Group>, Vec<Group>) =
            std::mem::take(&mut self.groups).into_iter().partition(|g| {
                self.persistent.contains(&g.id)
                    || self.voicers.values().any(|v| v.group == Some(g.id))
            });
        self.groups = keep;
        for g in gone {
            self.broadcast("voicechat:remove_group", p::remove_group(g.id));
        }
    }

    fn secret_for(&self, uuid: u128) -> Option<Vec<u8>> {
        let v = self.voicers.get(&uuid)?;
        Some(p::secret(&p::Secret {
            secret: v.secret,
            port: self.port,
            player: uuid,
            codec: self.cfg.codec_id(),
            mtu: self.cfg.mtu_size,
            distance: self.cfg.max_voice_distance,
            keep_alive: self.cfg.keep_alive,
            groups: self.cfg.enable_groups,
            voice_host: &self.cfg.voice_host,
            allow_recording: self.cfg.allow_recording,
        }))
    }

    /// The game thread's view of the players, once a tick: joins and leaves are told.
    pub fn tick(&mut self, guests: &[Guest]) -> Vec<Out> {
        self.tick_at(guests, Instant::now())
    }

    pub fn tick_at(&mut self, guests: &[Guest], now: Instant) -> Vec<Out> {
        for g in guests {
            if let Some(o) = self.online.get_mut(&g.uuid) {
                o.pos = g.pos;
                o.level = g.level;
                o.has_mod = g.has_mod;
                if o.dimension != g.dimension {
                    o.dimension = g.dimension.to_owned();
                }
                continue;
            }
            self.online.insert(
                g.uuid,
                Online {
                    name: g.name.to_owned(),
                    dimension: g.dimension.to_owned(),
                    pos: g.pos,
                    level: g.level,
                    has_mod: g.has_mod,
                    joined: now,
                    kicked: false,
                },
            );
            self.send(g.uuid, "minecraft:register", p::register_payload());
            self.broadcast_state(g.uuid);
        }
        let gone: Vec<u128> = self
            .online
            .keys()
            .filter(|u| !guests.iter().any(|g| g.uuid == **u))
            .copied()
            .collect();
        for uuid in gone {
            self.online.remove(&uuid);
            if self.voicers.remove(&uuid).is_some() {
                log(&format!("Disconnecting client {:032x}", uuid));
            }
            self.invites.retain(|(p, _)| *p != uuid);
            self.broadcast("voicechat:remove_state", p::remove_state(uuid));
            self.remove_empty_groups();
        }
        if self.cfg.force_voice_chat {
            let timeout = Duration::from_millis(self.cfg.login_timeout as u64);
            let mut late: Vec<u128> = self
                .online
                .iter()
                .filter(|(u, o)| {
                    !o.kicked
                        && !self.voicers.contains_key(*u)
                        && now.duration_since(o.joined) >= timeout
                })
                .map(|(u, _)| *u)
                .collect();
            late.sort();
            for player in late {
                self.online.get_mut(&player).unwrap().kicked = true;
                self.outbox.push(Out::Kick {
                    player,
                    text: MOD_REQUIRED.into(),
                });
            }
        }
        std::mem::take(&mut self.outbox)
    }

    /// A plugin channel message from a player.
    pub fn payload(&mut self, player: u128, channel: &str, data: &[u8], now: Instant) {
        if let Some(v) = self.voicers.get_mut(&player) {
            if now.duration_since(v.window.0) >= Duration::from_secs(1) {
                v.window = (now, 0);
            }
            v.window.1 += 1;
            if self.cfg.tcp_rate_limit > 0 && v.window.1 > self.cfg.tcp_rate_limit {
                return;
            }
        }
        let Ok(msg) = FromClient::decode(channel, data) else {
            return;
        };
        if !self.online.contains_key(&player) {
            return;
        }
        match msg {
            FromClient::RequestSecret { version } => {
                log(&format!(
                    "Received secret request of {} ({version})",
                    self.online[&player].name
                ));
                if version != p::COMPATIBILITY_VERSION {
                    self.outbox.push(Out::Chat {
                        player,
                        text: INCOMPATIBLE.into(),
                    });
                    return;
                }
                self.voicers.entry(player).or_insert_with(|| Voicer {
                    secret: random(),
                    addr: None,
                    connected: false,
                    last_seen: now,
                    disabled: false,
                    group: None,
                    window: (now, 1),
                });
                let mut all: Vec<u128> = self.online.keys().copied().collect();
                all.sort();
                let states: Vec<PlayerState> = all.into_iter().map(|u| self.state_of(u)).collect();
                self.send(player, "voicechat:states", p::states(&states));
                if self.cfg.enable_groups {
                    for g in self.groups.clone() {
                        self.send(player, "voicechat:add_group", p::add_group(&g));
                    }
                }
                let secret = self.secret_for(player).expect("just added");
                self.send(player, "voicechat:secret", secret);
            }
            _ if !self.voicers.contains_key(&player) => {}
            FromClient::UpdateState { disabled } => {
                self.voicers.get_mut(&player).unwrap().disabled = disabled;
                self.broadcast_state(player);
            }
            _ if !self.cfg.enable_groups || !self.may(player, "horizonvc.groups") => {}
            FromClient::CreateGroup {
                name,
                password,
                kind,
            } => {
                if !(p::NORMAL..=p::ISOLATED).contains(&kind) || name.trim().is_empty() {
                    return;
                }
                let mut id: [u8; 16] = random();
                id[6] = (id[6] & 0x0F) | 0x40;
                id[8] = (id[8] & 0x3F) | 0x80;
                let g = Group {
                    id: u128::from_be_bytes(id),
                    name,
                    password,
                    kind,
                };
                self.broadcast("voicechat:add_group", p::add_group(&g));
                self.groups.push(g.clone());
                self.voicers.get_mut(&player).unwrap().group = Some(g.id);
                self.broadcast_state(player);
                self.send(
                    player,
                    "voicechat:joined_group",
                    p::joined_group(Some(g.id), false),
                );
            }
            FromClient::SetGroup { id, password } => {
                let _ = self.join(player, id, password);
            }
            FromClient::LeaveGroup => self.leave(player),
        }
    }

    /// A voice chat player joins a group (`voicechat:set_group`, `/horizonvc join`, the API). An invitation to it
    /// stands in for the password.
    fn join(&mut self, player: u128, id: u128, password: Option<String>) -> Result<(), Join> {
        let Some(g) = self.groups.iter().find(|g| g.id == id) else {
            return Err(Join::NoGroup);
        };
        let invited = self.invites.remove(&(player, id));
        if !invited && g.password.is_some() && g.password != password {
            self.send(
                player,
                "voicechat:joined_group",
                p::joined_group(None, true),
            );
            return Err(Join::WrongPassword);
        }
        let Some(v) = self.voicers.get_mut(&player) else {
            return Err(Join::NoMod);
        };
        v.group = Some(id);
        self.broadcast_state(player);
        self.send(
            player,
            "voicechat:joined_group",
            p::joined_group(Some(id), false),
        );
        Ok(())
    }

    fn leave(&mut self, player: u128) {
        let Some(v) = self.voicers.get_mut(&player) else {
            return;
        };
        v.group = None;
        self.broadcast_state(player);
        self.send(
            player,
            "voicechat:joined_group",
            p::joined_group(None, false),
        );
        self.remove_empty_groups();
    }

    /// A datagram on the voice port; the answers to send, each to an address.
    pub fn datagram(
        &mut self,
        data: &[u8],
        from: SocketAddr,
        now: Instant,
    ) -> Vec<(SocketAddr, Vec<u8>)> {
        let mut out = Vec::new();
        let mut r = Reader::new(data);
        let (Ok(0xFF), Ok(uuid), Ok(sealed)) = (r.u8(), r.uuid(), r.bytes(65_536)) else {
            return out;
        };
        let mtu = self.cfg.mtu_size.max(0) as usize;
        let Some(v) = self.voicers.get_mut(&uuid) else {
            return out;
        };
        let Some(plain) = open(&v.secret, sealed) else {
            return out;
        };
        let Ok(packet) = Voice::decode(&plain, mtu) else {
            return out;
        };
        let secret = v.secret;
        match packet {
            Voice::Authenticate { player, secret: s } if player == uuid && s == secret => {
                v.addr = Some(from);
                v.last_seen = now;
                out.push((from, frame(&secret, &[id::AUTHENTICATE_ACK])));
                log(&format!("Successfully authenticated player {uuid:032x}"));
            }
            _ if v.addr.is_none() => {}
            Voice::KeepAlive => v.last_seen = now,
            Voice::ConnectionCheck => {
                v.last_seen = now;
                out.push((from, frame(&secret, &[id::CONNECTION_CHECK_ACK])));
                if !v.connected {
                    v.connected = true;
                    let name = self
                        .online
                        .get(&uuid)
                        .map(|o| o.name.clone())
                        .unwrap_or_default();
                    log(&format!(
                        "Player {name} ({uuid:032x}) successfully connected to voice chat"
                    ));
                    self.broadcast_state(uuid);
                }
            }
            Voice::Mic {
                audio,
                sequence,
                whispering,
            } if v.connected => {
                v.last_seen = now;
                if self.may(uuid, "horizonvc.speak")
                    && !self
                        .mic_listeners
                        .iter()
                        .any(|l| l(uuid, audio, whispering))
                {
                    out.extend(self.route(uuid, audio, sequence, whispering));
                }
            }
            _ => {}
        }
        out
    }

    fn listener(&self, uuid: u128) -> Option<Listener<'_>> {
        self.voicers.get(&uuid)?;
        self.view(uuid)
    }

    /// An online player as routing sees them, with or without the mod (an API audio sender speaks for a player
    /// without it). A player without `horizonvc.listen` counts as having their voice chat off.
    fn view(&self, uuid: u128) -> Option<Listener<'_>> {
        let o = self.online.get(&uuid)?;
        let v = self.voicers.get(&uuid);
        let group = v
            .and_then(|v| v.group)
            .and_then(|id| self.groups.iter().find(|g| g.id == id))
            .map(|g| (g.id, g.kind));
        Some(Listener {
            uuid,
            dimension: &o.dimension,
            pos: o.pos,
            disabled: v.is_some_and(|v| v.disabled) || !self.may(uuid, "horizonvc.listen"),
            group,
        })
    }

    fn route(
        &self,
        speaker: u128,
        audio: &[u8],
        sequence: i64,
        whispering: bool,
    ) -> Vec<(SocketAddr, Vec<u8>)> {
        let Some(me) = self.view(speaker) else {
            return Vec::new();
        };
        let all: Vec<Listener> = self
            .voicers
            .iter()
            .filter(|(_, v)| v.connected)
            .filter_map(|(u, _)| self.listener(*u))
            .collect();
        let distance = (if whispering {
            self.cfg.whisper_distance
        } else {
            self.cfg.max_voice_distance
        }) as f32;
        recipients(&me, &all, self.cfg.cutoff())
            .into_iter()
            .filter_map(|(to, how)| {
                let v = &self.voicers[&to];
                let proximity = (how == Delivery::Proximity).then_some((distance, whispering));
                Some((
                    v.addr?,
                    frame(&v.secret, &p::sound(speaker, audio, sequence, proximity)),
                ))
            })
            .collect()
    }

    /// Keep-alives to every authenticated client; those silent for ten intervals lose
    /// their voice connection and are sent their secret again to reconnect.
    pub fn keep_alive(&mut self, now: Instant) -> Vec<(SocketAddr, Vec<u8>)> {
        let limit = Duration::from_millis(self.cfg.keep_alive as u64 * 10);
        let mut out = Vec::new();
        let mut lost = Vec::new();
        for (uuid, v) in &mut self.voicers {
            let Some(addr) = v.addr else { continue };
            if now.duration_since(v.last_seen) > limit {
                v.addr = None;
                v.connected = false;
                lost.push(*uuid);
            } else {
                out.push((addr, frame(&v.secret, &[id::KEEP_ALIVE])));
            }
        }
        lost.sort();
        for uuid in lost {
            log(&format!("Player {uuid:032x} timed out"));
            self.broadcast_state(uuid);
            if let Some(s) = self.secret_for(uuid) {
                self.send(uuid, "voicechat:secret", s);
            }
        }
        out
    }
}

/// Why a join failed.
#[derive(Debug, PartialEq, Eq)]
enum Join {
    NoGroup,
    WrongPassword,
    NoMod,
}

/// A server datagram: 0xFF, then the sealed packet with a VarInt length.
fn frame(secret: &[u8; 16], plain: &[u8]) -> Vec<u8> {
    let mut w = Writer::default();
    w.u8(0xFF).bytes(&seal(secret, plain));
    w.0
}

/// Opens the voice socket and starts its thread, once; later calls do nothing.
fn start(state: &mut Option<State>) {
    if state.is_some() {
        return;
    }
    let cfg = Config::load(Path::new(CONFIG_PATH)).unwrap_or_else(|e| {
        log_warn(&format!("Using the default config: {e}"));
        Config::default()
    });
    let props = std::fs::read_to_string("server.properties").unwrap_or_default();
    let port = if cfg.port < 0 {
        server_property(&props, "server-port")
            .and_then(|p| p.parse().ok())
            .unwrap_or(25565)
    } else {
        cfg.port
    };
    let host = match cfg.bind_address.as_str() {
        "" => server_property(&props, "server-ip")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "0.0.0.0".into()),
        "*" => "0.0.0.0".into(),
        other => other.to_owned(),
    };
    let mut s = State::new(cfg, port);
    match UdpSocket::bind((host.as_str(), port as u16)) {
        Ok(sock) => {
            let _ = sock.set_read_timeout(Some(Duration::from_millis(50)));
            match sock.try_clone() {
                Ok(thread_sock) => {
                    s.socket = Some(sock);
                    std::thread::Builder::new()
                        .name("horizonvc-udp".into())
                        .spawn(move || udp_loop(thread_sock))
                        .expect("spawning the voice thread");
                    log(&format!("Voice server is listening on UDP port {port}"));
                }
                Err(e) => log_warn(&format!("Voice server could not start: {e}")),
            }
        }
        Err(e) => log_warn(&format!(
            "Voice server could not bind {host}:{port}: {e}"
        )),
    }
    *state = Some(s);
}

fn udp_loop(sock: UdpSocket) {
    let mut buf = vec![0u8; 65_536];
    let mut last_keep_alive = Instant::now();
    loop {
        let mut out = Vec::new();
        if let Ok((n, from)) = sock.recv_from(&mut buf) {
            if let Some(s) = lock().as_mut() {
                out = s.datagram(&buf[..n], from, Instant::now());
            }
        }
        let now = Instant::now();
        if let Some(s) = lock().as_mut() {
            if now.duration_since(last_keep_alive) >= Duration::from_millis(s.cfg.keep_alive as u64)
            {
                last_keep_alive = now;
                out.extend(s.keep_alive(now));
            }
        }
        for (to, data) in out {
            let _ = sock.send_to(&data, to);
        }
    }
}

/// The game thread, once a tick: the players on the server, and what to send them.
pub fn tick(guests: &[Guest]) -> Vec<Out> {
    let mut state = lock();
    start(&mut state);
    state.as_mut().map(|s| s.tick(guests)).unwrap_or_default()
}

/// Runs `f` on the voice server's state once it has started.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    lock().as_mut().map(f)
}

/// The game thread: a player's command line (without its slash); `None` when it isn't `/horizonvc`. What it
/// sends other players goes out with the next tick.
pub fn command(player: u128, line: &str) -> Option<Vec<String>> {
    if !commands::is_command_word(line.split_whitespace().next()?) {
        return None;
    }
    with_state(|s| s.command(player, line))
        .unwrap_or_else(|| Some(vec!["The voice chat server is not running.".to_string()]))
}

/// Whether `player`'s client registered a `voicechat:` channel (has the voice chat mod), as of the last tick.
pub fn client_has_mod(player: u128) -> bool {
    with_state(|s| s.online.get(&player).is_some_and(|o| o.has_mod)).unwrap_or(false)
}

/// The game thread: a player's plugin channel message on a `voicechat:` channel.
pub fn on_payload(player: u128, channel: &str, data: &[u8]) {
    if let Some(s) = lock().as_mut() {
        s.payload(player, channel, data, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u128 = 1;
    const B: u128 = 2;

    fn guests(xb: f64) -> Vec<Guest<'static>> {
        vec![
            Guest {
                uuid: A,
                name: "A",
                dimension: "minecraft:overworld",
                pos: [0.5, -60.0, 0.5],
                level: 0,
                has_mod: false,
            },
            Guest {
                uuid: B,
                name: "B",
                dimension: "minecraft:overworld",
                pos: [xb, -60.0, 0.5],
                level: 0,
                has_mod: false,
            },
        ]
    }

    fn channels(out: &[Out], to: u128) -> Vec<&'static str> {
        out.iter()
            .filter_map(|o| match o {
                Out::Payload {
                    player, channel, ..
                } if *player == to => Some(*channel),
                _ => None,
            })
            .collect()
    }

    fn client_frame(uuid: u128, secret: &[u8; 16], plain: &[u8]) -> Vec<u8> {
        let mut w = Writer::default();
        w.u8(0xFF).uuid(uuid).bytes(&seal(secret, plain));
        w.0
    }

    /// Joins both, gives both a voice connection; returns their secrets.
    fn connected(s: &mut State, now: Instant) -> [[u8; 16]; 2] {
        s.tick(&guests(2.5));
        let mut secrets = [[0; 16]; 2];
        for (i, u) in [A, B].into_iter().enumerate() {
            s.payload(u, "voicechat:request_secret", &20i32.to_be_bytes(), now);
            secrets[i] = s.voicers[&u].secret;
            let addr: SocketAddr = format!("127.0.0.1:{}", 40000 + i).parse().unwrap();
            let mut auth = vec![id::AUTHENTICATE];
            auth.extend(u.to_be_bytes());
            auth.extend(secrets[i]);
            assert_eq!(
                s.datagram(&client_frame(u, &secrets[i], &auth), addr, now)
                    .len(),
                1
            );
            assert_eq!(
                s.datagram(
                    &client_frame(u, &secrets[i], &[id::CONNECTION_CHECK]),
                    addr,
                    now
                )
                .len(),
                1
            );
        }
        s.tick(&guests(2.5));
        secrets
    }

    #[test]
    fn a_join_registers_the_channels_and_a_secret_request_answers_in_order() {
        let mut s = State::new(Config::default(), 25666);
        let out = s.tick(&guests(2.5));
        assert_eq!(channels(&out, A), vec!["minecraft:register"]);
        s.payload(
            A,
            "voicechat:request_secret",
            &1i32.to_be_bytes(),
            Instant::now(),
        );
        assert!(matches!(
            &s.tick(&guests(2.5))[..],
            [Out::Chat { player: A, .. }]
        ));
        s.payload(
            A,
            "voicechat:request_secret",
            &20i32.to_be_bytes(),
            Instant::now(),
        );
        assert_eq!(
            channels(&s.tick(&guests(2.5)), A),
            vec!["voicechat:states", "voicechat:secret"]
        );
    }

    #[test]
    fn mic_reaches_the_listener_in_range_only() {
        let now = Instant::now();
        let mut s = State::new(Config::default(), 25666);
        let [sa, sb] = connected(&mut s, now);
        let mut mic = vec![id::MIC, 2, 7, 7];
        mic.extend(5i64.to_be_bytes());
        mic.push(0);
        let from: SocketAddr = "127.0.0.1:40000".parse().unwrap();
        let out = s.datagram(&client_frame(A, &sa, &mic), from, now);
        assert_eq!(out.len(), 1);
        let (to, data) = &out[0];
        assert_eq!(to.port(), 40001);
        let mut r = Reader::new(data);
        assert_eq!(r.u8(), Ok(0xFF));
        let plain = open(&sb, r.bytes(9999).unwrap()).unwrap();
        assert_eq!(plain, p::sound(A, &[7, 7], 5, Some((48.0, false))));
        s.tick(&guests(60.5));
        assert!(s
            .datagram(&client_frame(A, &sa, &mic), from, now)
            .is_empty());
    }

    #[test]
    fn groups_and_their_cleanup() {
        let now = Instant::now();
        let mut s = State::new(Config::default(), 25666);
        connected(&mut s, now);
        let mut create = Writer::default();
        create.string("g").bool(true).string("pw").i16(p::NORMAL);
        s.payload(A, "voicechat:create_group", &create.0, now);
        let out = s.tick(&guests(2.5));
        assert_eq!(
            channels(&out, A),
            vec![
                "voicechat:add_group",
                "voicechat:state",
                "voicechat:joined_group"
            ]
        );
        assert_eq!(
            channels(&out, B),
            vec!["voicechat:add_group", "voicechat:state"]
        );
        let id = s.groups[0].id;
        let mut wrong = Writer::default();
        wrong.uuid(id).bool(false);
        s.payload(B, "voicechat:set_group", &wrong.0, now);
        assert_eq!(
            s.tick(&guests(2.5)),
            vec![Out::Payload {
                player: B,
                channel: "voicechat:joined_group",
                data: p::joined_group(None, true)
            }]
        );
        let mut right = Writer::default();
        right.uuid(id).bool(true).string("pw");
        s.payload(B, "voicechat:set_group", &right.0, now);
        s.payload(B, "voicechat:leave_group", &[], now);
        s.tick(&guests(2.5));
        assert_eq!(s.groups.len(), 1, "A is still in it");
        s.payload(A, "voicechat:leave_group", &[], now);
        assert!(channels(&s.tick(&guests(2.5)), B).contains(&"voicechat:remove_group"));
        assert!(s.groups.is_empty());
    }

    #[test]
    fn a_silent_client_times_out_and_gets_its_secret_again() {
        let now = Instant::now();
        let mut s = State::new(Config::default(), 25666);
        connected(&mut s, now);
        assert_eq!(s.keep_alive(now + Duration::from_secs(5)).len(), 2);
        assert!(s.keep_alive(now + Duration::from_secs(11)).is_empty());
        let out = s.tick(&guests(2.5));
        assert!(channels(&out, A).contains(&"voicechat:secret"));
        assert!(!s.voicers[&A].connected);
    }

    #[test]
    fn force_voice_chat_kicks_after_the_timeout() {
        let now = Instant::now();
        let mut s = State::new(
            Config {
                force_voice_chat: true,
                login_timeout: 1000,
                ..Config::default()
            },
            25666,
        );
        s.tick_at(&guests(2.5), now);
        s.payload(A, "voicechat:request_secret", &20i32.to_be_bytes(), now);
        s.tick_at(&guests(2.5), now + Duration::from_millis(500));
        let out = s.tick_at(&guests(2.5), now + Duration::from_millis(1000));
        assert_eq!(
            out,
            vec![Out::Kick {
                player: B,
                text: MOD_REQUIRED.into()
            }]
        );
        assert!(s
            .tick_at(&guests(2.5), now + Duration::from_secs(5))
            .is_empty());
    }

    #[test]
    fn commands_join_invite_leave_and_test() {
        let now = Instant::now();
        let mut s = State::new(Config::default(), 25666);
        connected(&mut s, now);
        let mut create = Writer::default();
        create.string("team").bool(true).string("pw").i16(p::NORMAL);
        s.payload(A, "voicechat:create_group", &create.0, now);
        s.tick(&guests(2.5));
        let id = s.groups[0].id;
        assert_eq!(
            s.command(B, "horizonvc join team"),
            Some(vec!["Wrong password.".into()])
        );
        assert_eq!(
            s.command(B, "horizonvc join team pw"),
            Some(vec!["You joined the group team.".into()])
        );
        assert_eq!(s.voicers[&B].group, Some(id));
        assert_eq!(
            s.command(B, "horizonvc leave"),
            Some(vec!["You left the group.".into()])
        );
        s.tick(&guests(2.5));
        // An invitation stands in for the password, once.
        assert_eq!(
            s.command(A, "horizonvc invite b"),
            Some(vec!["Invited B to team.".into()])
        );
        let out = s.tick(&guests(2.5));
        let want = format!("/horizonvc join {}", commands::uuid_string(id));
        assert!(out
            .iter()
            .any(|o| matches!(o, Out::ChatCommand { player: B, command, .. } if *command == want)));
        assert_eq!(
            s.command(B, &want[1..]),
            Some(vec!["You joined the group team.".into()])
        );
        assert_eq!(
            s.command(B, "horizonvc test A"),
            Some(vec!["You don't have permission to do this.".into()])
        );
        s.online.get_mut(&B).unwrap().level = 4;
        assert_eq!(
            s.command(B, "horizonvc test a"),
            Some(vec!["A is connected to voice chat.".into()])
        );
        assert_eq!(s.command(B, "gamemode 1"), None);
        assert_eq!(s.command(B, "horizonvc").unwrap().len(), 6);
        assert_eq!(s.command(B, "hvc").unwrap().len(), 6);
        assert!(s.command(B, "voicechat").is_none());
    }

    #[test]
    fn persistent_groups_stay_and_permissions_apply() {
        let now = Instant::now();
        let mut s = State::new(Config::default(), 25666);
        let [sa, _] = connected(&mut s, now);
        s.persistent.insert(77);
        s.groups.push(Group {
            id: 77,
            name: "kept".into(),
            password: None,
            kind: p::OPEN,
        });
        assert!(s.join(A, 77, None).is_ok());
        s.leave(A);
        assert_eq!(s.groups.len(), 1);
        fn mute_a(player: u128, _: u8, node: &str) -> bool {
            !(player == A && node == "horizonvc.speak")
        }
        s.permissions = mute_a;
        let mut mic = vec![id::MIC, 1, 7];
        mic.extend(5i64.to_be_bytes());
        mic.push(0);
        let from: SocketAddr = "127.0.0.1:40000".parse().unwrap();
        assert!(s
            .datagram(&client_frame(A, &sa, &mic), from, now)
            .is_empty());
        s.permissions = default_permissions;
        s.mic_listeners.push(Box::new(|speaker: u128, _: &[u8], _: bool| speaker == A));
        assert!(s
            .datagram(&client_frame(A, &sa, &mic), from, now)
            .is_empty());
        s.mic_listeners.clear();
        assert_eq!(s.datagram(&client_frame(A, &sa, &mic), from, now).len(), 1);
    }

    #[test]
    fn leaving_the_server_removes_the_state() {
        let now = Instant::now();
        let mut s = State::new(Config::default(), 25666);
        connected(&mut s, now);
        let out = s.tick(&guests(2.5)[..1]);
        assert_eq!(
            out,
            vec![Out::Payload {
                player: A,
                channel: "voicechat:remove_state",
                data: p::remove_state(B)
            }]
        );
    }

    #[test]
    fn the_guests_channels_mark_the_client() {
        let mut s = State::new(Config::default(), 25666);
        let mut g = guests(2.5);
        s.tick_at(&g, Instant::now());
        assert!(!s.online[&A].has_mod);
        g[0].has_mod = true;
        s.tick_at(&g, Instant::now());
        assert!(s.online[&A].has_mod && !s.online[&B].has_mod);
    }
}
