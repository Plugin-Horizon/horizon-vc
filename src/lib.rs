//! HorizonVC (the Simple Voice Chat server) as an arismoko native trait mod on `mod_api`. The voice
//! server is the unchanged `horizonvc-core` crate (core/): protocol, UDP thread, groups, commands.
//! This file only wires it to the engine: the players each tick, the commands, the plugin channel
//! payloads and the messages to players.
//!
//! Plugin channels: the twelve `voicechat:*` channels are declared as raw play channels
//! (`payload_handlers().registrar("1").play_raw`, arismoko #3209), the sends are
//! `PlayerRef::send_payload`, the client's announced channels (`minecraft:register`) give `has_mod`,
//! and a `Kick` really kicks.
//!
//! Config: the original `plugins/voicechat/voicechat-server.properties`, read by the core itself (and
//! `config/horizonvc/horizonvc-server.properties` where that is missing), so no config hook is needed.

pub mod horizonvc_core;
use crate::horizonvc_core::{command, on_payload, tick, Guest, Out, USAGES};
use mod_api::{
    Command, CommandArgumentValue, CommandSource, Flow, Hooks, PlayerRef, RegisterCommandsEvent, RegisterNodeCommandsEvent,
    Registrar, RegistrationContext, RunModCommandEvent, ServerHost, ServerTickEvent, ServerTickPhase,
    Text,
};
use mod_api::text::{TextClick, TextStyle};
use mod_api::{IPayloadContext, IPayloadHandler, ModServiceCallEvent, ModServiceError, PayloadBytes, PayloadId};
use std::sync::Arc;

/// The root words of the usages in `USAGES`: `/voicechat` and its aliases `/horizonvc`, `/hvc`.
const ROOTS: [&str; 3] = ["voicechat", "horizonvc", "hvc"];

#[derive(Default)]
struct Mod;

/// A real player as the tick sees them. The core's `Guest` borrows the name and dimension from here.
struct Seen {
    player: PlayerRef,
    uuid: u128,
    name: String,
    dimension: String,
    pos: [f64; 3],
    level: u8,
    spectator: bool,
    watching: Option<u128>,
}

fn log(line: &str) {
    eprintln!("[HorizonVC] {line}");
}

/// The `voicechat.*` nodes (the documented names) the engine's permission provider (LuckPerms) is asked for.
const NODES: [&str; 4] = ["voicechat.admin", "voicechat.listen", "voicechat.speak", "voicechat.groups"];

/// A voice chat node as a typed Boolean permission node.
fn typed_node(name: &str) -> mod_api::PermissionNode {
    mod_api::PermissionNode::new(name, mod_api::PermissionValueType::Boolean)
}

/// The operator level that grants a node by default: `voicechat.admin` from level 1, the others to everyone.
fn default_level(name: &str) -> mod_api::PermissionLevel {
    mod_api::PermissionLevel::from_level(i32::from(name == "voicechat.admin"))
}

/// The engine's answers per player, read in `run_mod_command` (which has the host) for the core's
/// permission check, which is a plain fn without a host. Lock order: this list is never held while the core runs.
static NODE_ANSWERS: std::sync::Mutex<Vec<(u128, &'static str, bool)>> = std::sync::Mutex::new(Vec::new());

/// The core's permission check: the provider's answer read for this player, else the core's default
/// (`voicechat.admin` needs operator level 1, the other nodes are everyone's).
fn node_answer(player: u128, level: u8, node: &str) -> bool {
    if let Ok(answers) = NODE_ANSWERS.lock() {
        if let Some((_, _, answer)) = answers.iter().find(|(p, n, _)| *p == player && *n == node) {
            return *answer;
        }
    }
    crate::horizonvc_core::server::default_permissions(player, level, node)
}

/// The online real players (no fakes), with the reads the core needs. A player missing a name,
/// dimension or position is left out for this tick.
fn seen(host: &mut dyn ServerHost) -> Vec<Seen> {
    let mut out = Vec::new();
    for player in host.players().server_players() {
        if player.is_fake(&*host) != Some(false) {
            continue;
        }
        let (Some(name), Some(dimension), Some(pos)) = (
            player.name(&*host),
            player.dimension(&*host),
            player.entity().position(&*host),
        ) else {
            continue;
        };
        let spectator = player.is_spectator(&*host).unwrap_or(false);
        // The player a spectator watches: the spectated entity's uuid (only players count in the core).
        let watching = if spectator {
            player.spectator_target(&*host).flatten().and_then(|e| e.uuid(host))
        } else {
            None
        };
        out.push(Seen {
            player,
            uuid: player.uuid(),
            name,
            dimension: dimension.to_string(),
            pos,
            level: player.permission_level(&*host).unwrap_or(0),
            spectator,
            watching,
        });
    }
    out
}

/// The raw channels of the voice chat protocol: the ones a client sends, and the ones the server sends.
mod raw {
    use super::*;

    pub const SERVERBOUND: [&str; 5] = [
        "voicechat:request_secret",
        "voicechat:create_group",
        "voicechat:leave_group",
        "voicechat:set_group",
        "voicechat:update_state",
    ];
    pub const CLIENTBOUND: [&str; 9] = [
        "voicechat:state",
        "voicechat:states",
        "voicechat:remove_state",
        "voicechat:add_group",
        "voicechat:remove_group",
        "voicechat:joined_group",
        "voicechat:secret",
        "voicechat:add_category",
        "voicechat:remove_category",
    ];

    /// One serverbound channel: the bytes a client sent on it go to the core, as `net_from_client` did.
    pub struct Inbound(pub &'static str);

    impl IPayloadHandler for Inbound {
        fn handle(
            &self,
            payload: PayloadBytes,
            context: &mut dyn IPayloadContext,
        ) -> Result<(), mod_api::PayloadHandleError> {
            let player = context
                .player()
                .map_err(|_| mod_api::PayloadHandleError::Failed("no sending player".to_string()))?;
            on_payload(player.uuid(), self.0, &payload.0);
            Ok(())
        }
    }

    pub fn register(registrar: &mut dyn Registrar) {
        let mut event = registrar.payload_handlers();
        let mut raw = event.registrar("1");
        for channel in SERVERBOUND {
            raw.play_raw(PayloadId(channel.into()), Some(Arc::new(Inbound(channel))));
        }
        for channel in CLIENTBOUND {
            raw.play_raw(PayloadId(channel.into()), None);
        }
    }

    /// Whether the client announced a `voicechat:` channel (`minecraft:register`).
    pub fn has_mod(host: &dyn ServerHost, player: PlayerRef) -> bool {
        player
            .announced_channels(host)
            .is_some_and(|channels| channels.iter().any(|c| c.starts_with("voicechat:")))
    }

    /// A server message on its raw channel, to the player `uuid` if online.
    pub fn send(host: &mut dyn ServerHost, players: &[Seen], uuid: u128, channel: &str, data: Vec<u8>) {
        // `minecraft:register` (the core's channel list for the client) is the engine's: mods may not send on
        // `minecraft:*`, and the engine does not announce declared raw channels. Skipped quietly.
        if channel.starts_with("minecraft:") {
            return;
        }
        let Some(p) = players.iter().find(|s| s.uuid == uuid) else {
            return;
        };
        if p.player.send_payload(host, &PayloadId(channel.into()), PayloadBytes(data)).is_err() {
            log(&format!("{channel} not sent to {}: refused", p.name));
        }
    }
}

/// A chat line to the player `uuid`, if they are still online.
fn tell(host: &mut dyn ServerHost, players: &[Seen], uuid: u128, text: String) {
    if let Some(p) = players.iter().find(|s| s.uuid == uuid) {
        let _ = p.player.tell(host, Text::literal(text));
    }
}

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn permission_gather_nodes(&self, event: &mut mod_api::PermissionGatherNodesEvent<'_>) {
        for node in NODES {
            let _ = event.nodes.add(typed_node(node), mod_api::NodeDefault::Level(default_level(node)));
        }
    }

    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        raw::register(registrar);
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        for (usage, level) in USAGES {
            if !usage.contains(" test ") {
                event.commands.push(Command::op(usage, level));
            }
        }
    }

    fn register_node_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterNodeCommandsEvent<'_>) {
        // `test` is for `voicechat.admin`: the node decides who sees and runs it when a provider is set.
        for (usage, level) in USAGES {
            if usage.contains(" test ") {
                event.commands.push(Command::op(usage, level).with_node(&typed_node("voicechat.admin")));
            }
        }
    }

    fn run_mod_command(&self, host: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
        if !event.command.literals.first().is_some_and(|root| ROOTS.contains(&root.as_str())) {
            return Flow::Continue;
        }
        let CommandSource::Player { player, .. } = &event.source else {
            let _ = host
                .commands()
                .feedback(Text::literal("The voice chat command is for players."));
            return Flow::Cancel;
        };
        let uuid = player.uuid();
        // The core parses the typed line (`/voicechat join <group> [password]`), so the parsed
        // invocation is put back into that form: literals, then the values that have text. A player
        // argument goes back as its name; the core matches names as the original does.
        let mut line = event.command.literals.join(" ");
        for argument in &event.command.arguments {
            let text = match &argument.value {
                CommandArgumentValue::String(s) => s.clone(),
                CommandArgumentValue::Player(p) => p.name(&*host).unwrap_or_default(),
                _ => continue,
            };
            line.push(' ');
            line.push_str(&text);
        }
        // The provider's answer for each voice chat node, for the core's permission check (see node_answer).
        // Any refusal of the typed query is a denial, never a grant.
        let answers_now: Vec<(u128, &str, bool)> = NODES
            .iter()
            .map(|node| (uuid, *node, player.permission_bool(&mut *host, &typed_node(node)).unwrap_or(false)))
            .collect();
        if let Ok(mut answers) = NODE_ANSWERS.lock() {
            answers.retain(|(p, _, _)| *p != uuid);
            answers.extend(answers_now);
        }
        crate::horizonvc_core::api::set_permission_check(node_answer);
        for answer in command(uuid, &line).unwrap_or_default() {
            let _ = host.commands().feedback(Text::literal(answer));
        }
        Flow::Cancel
    }

    /// The service other mods call: `voicechat:api` (`horizonvc_core::api::handle`).
    fn mod_service_call(&self, _: &mut dyn ServerHost, event: &mut ModServiceCallEvent<'_>) -> Option<PayloadBytes> {
        if event.service != "voicechat:api" {
            return None;
        }
        let text = std::str::from_utf8(&event.payload.0).ok()?;
        crate::horizonvc_core::api::handle(text).map(|reply| PayloadBytes(reply.into_bytes()))
    }

    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if !matches!(event.phase, ServerTickPhase::ServerTickEnd { .. }) {
            return;
        }
        let players = seen(host);
        let read: &dyn ServerHost = &*host;
        let guests: Vec<Guest> = players
            .iter()
            .map(|s| Guest {
                uuid: s.uuid,
                name: &s.name,
                dimension: &s.dimension,
                pos: s.pos,
                level: s.level,
                spectator: s.spectator,
                watching: s.watching,
                // The channels the client announced in `minecraft:register` decide.
                has_mod: raw::has_mod(read, s.player),
            })
            .collect();
        let out = tick(&guests);
        drop(guests);
        for o in out {
            match o {
                Out::Payload { player, channel, data } => {
                    raw::send(host, &players, player, channel, data);
                }
                Out::Chat { player, text } => tell(host, &players, player, text),
                Out::ChatCommand { player, text, command } => {
                    // The command part is a run_command click, as the Java plugin's invite: the player who
                    // clicks it runs `command` (it starts with a slash, as the core writes it).
                    if let Some(s) = players.iter().find(|s| s.uuid == player) {
                        let mut click = TextStyle::default();
                        click.click = Some(TextClick::RunCommand(command.clone()));
                        let line = Text::literal(format!("{text} ")).append(Text::literal(command).with_style(click));
                        let _ = s.player.tell(host, line);
                    }
                }
                Out::Kick { player, text } => {
                    // has_mod is the announced channels, so only a client without the mod is kicked.
                    if let Some(s) = players.iter().find(|s| s.uuid == player) {
                        let _ = s.player.disconnect(host, Text::literal(text));
                    }
                }
            }
        }
        deliver_events(host);
    }
}

/// Tells the mods that subscribed what the voice chat did since the last tick: each gets one call of its service
/// `<id>:voicechat_event` per event line. A mod that is gone is dropped from the list.
fn deliver_events(host: &mut dyn ServerHost) {
    use crate::horizonvc_core::api;
    let lines = api::take_events();
    if lines.is_empty() {
        return;
    }
    for id in api::subscribers() {
        let service = format!("{id}:voicechat_event");
        for line in &lines {
            if let Err(ModServiceError::ProviderNotLoaded) = host.commands().call_mod_service(&service, line.as_bytes()) {
                api::unsubscribe(&id);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;

    #[test]
    fn the_sub_commands_of_the_original_are_in_the_tree() {
        for root in ROOTS {
            for want in ["help", "test <player:player>", "invite <player:player>", "join <group:word> [password:text]", "leave"] {
                assert!(USAGES.iter().any(|(u, _)| *u == format!("{root} {want}")), "{root} {want}");
            }
            assert!(USAGES.iter().any(|(u, _)| *u == root), "{root}");
        }
    }
}
