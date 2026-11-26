//! `/voicechat help|test|invite|join|leave` (the original's command name), with the aliases `/horizonvc` and `/hvc`.
use super::{Join, Out, State};

/// The usages to register, with the operator level each needs to be seen (0: everyone; `test` also carries the `voicechat.admin` node). `test` needs
/// `voicechat.admin`, checked when it runs.
pub const USAGES: [(&str, u8); 18] = [
    ("voicechat", 0),
    ("voicechat help", 0),
    ("voicechat test <player:player>", 1),
    ("voicechat invite <player:player>", 0),
    ("voicechat join <group:word> [password:text]", 0),
    ("voicechat leave", 0),
    ("horizonvc", 0),
    ("horizonvc help", 0),
    ("horizonvc test <player:player>", 1),
    ("horizonvc invite <player:player>", 0),
    ("horizonvc join <group:word> [password:text]", 0),
    ("horizonvc leave", 0),
    ("hvc", 0),
    ("hvc help", 0),
    ("hvc test <player:player>", 1),
    ("hvc invite <player:player>", 0),
    ("hvc join <group:word> [password:text]", 0),
    ("hvc leave", 0),
];

/// Whether the first word of a command line is `/voicechat` or one of its aliases `/horizonvc`, `/hvc`.
pub fn is_command_word(word: &str) -> bool {
    ["voicechat", "horizonvc", "hvc"]
        .iter()
        .any(|name| word.eq_ignore_ascii_case(name))
}

const HELP: [&str; 6] = [
    "/voicechat help - List these commands (aliases /horizonvc, /hvc)",
    "/voicechat test <player> - Check whether a player's voice chat is connected",
    "/voicechat invite <player> - Send a player a link to your group",
    "/voicechat join <group> [password] - Join the group of that name",
    "/voicechat join <group id> - Join the group with that ID",
    "/voicechat leave - Leave your current group",
];

/// `8-4-4-4-12` hex digits.
fn parse_uuid(s: &str) -> Option<u128> {
    let b = s.as_bytes();
    if b.len() != 36 || [8, 13, 18, 23].iter().any(|&i| b[i] != b'-') {
        return None;
    }
    u128::from_str_radix(&s.replace('-', ""), 16).ok()
}

pub fn uuid_string(v: u128) -> String {
    let h = format!("{v:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

impl State {
    fn online_named(&self, name: &str) -> Option<u128> {
        self.online
            .iter()
            .find(|(_, o)| o.name.eq_ignore_ascii_case(name))
            .map(|(u, _)| *u)
    }

    /// A player's command line (without its slash); `None` when it isn't `/voicechat`. Lines for other players
    /// go to the outbox.
    pub fn command(&mut self, player: u128, line: &str) -> Option<Vec<String>> {
        let mut words = line.split_whitespace();
        if !is_command_word(words.next()?) {
            return None;
        }
        let sub = words.next().map(str::to_ascii_lowercase);
        let args: Vec<&str> = words.collect();
        let reply = |s: &str| Some(vec![s.to_string()]);
        // The original answers every form, help included, with this line to a client that did not announce the mod.
        if !self.voicers.contains_key(&player) {
            return reply(&self.texts.command_needed());
        }
        let sub = match sub.as_deref() {
            None | Some("help") => return Some(HELP.iter().map(|s| s.to_string()).collect()),
            Some(s) => s.to_string(),
        };
        match (sub.as_str(), &args[..]) {
            ("test", [name]) => {
                if !self.may(player, "voicechat.admin") {
                    return reply("You don't have permission to do this.");
                }
                let Some(target) = self.online_named(name) else {
                    return reply(&format!("Player {name} not found."));
                };
                let name = self.online[&target].name.clone();
                reply(&match self.voicers.get(&target) {
                    None => format!("{name} doesn't have voice chat installed."),
                    Some(v) if !v.connected => {
                        format!("{name} has voice chat installed but is not connected.")
                    }
                    Some(_) => format!("{name} is connected to voice chat."),
                })
            }
            ("invite" | "join" | "leave", _) if !self.cfg.enable_groups => {
                reply("Groups are disabled on this server.")
            }
            ("invite" | "join" | "leave", _) if !self.may(player, "voicechat.groups") => {
                reply("You don't have permission to use groups.")
            }
            ("invite", [name]) => {
                let Some(group) = self.voicers[&player].group else {
                    return reply("You are not in a group.");
                };
                let Some(target) = self.online_named(name) else {
                    return reply(&format!("Player {name} not found."));
                };
                let target_name = self.online[&target].name.clone();
                if !self.voicers.contains_key(&target) {
                    return reply(&format!("{target_name} doesn't have voice chat installed."));
                }
                let group_name = self.groups.iter().find(|g| g.id == group)?.name.clone();
                self.invites.insert((target, group));
                self.outbox.push(Out::ChatCommand {
                    player: target,
                    text: format!(
                        "{} invited you to the group {group_name}. [Click to join]",
                        self.online[&player].name
                    ),
                    command: format!("/voicechat join {}", uuid_string(group)),
                });
                reply(&format!("Invited {target_name} to {group_name}."))
            }
            ("join", [first, rest @ ..]) => {
                // A UUID joins by ID; otherwise the longest leading run of words that names a group is the
                // name (group names may contain spaces) and the words after it are the password.
                let mut password = (!rest.is_empty()).then(|| rest.join(" "));
                let mut group = first.to_string();
                let id = match parse_uuid(first) {
                    Some(id) => id,
                    None => {
                        let mut found: Vec<u128> = Vec::new();
                        for n in (1..=args.len()).rev() {
                            let name = args[..n].join(" ");
                            let exact: Vec<u128> =
                                self.groups.iter().filter(|g| g.name == name).map(|g| g.id).collect();
                            found = if exact.is_empty() {
                                self.groups
                                    .iter()
                                    .filter(|g| g.name.eq_ignore_ascii_case(&name))
                                    .map(|g| g.id)
                                    .collect()
                            } else {
                                exact
                            };
                            if !found.is_empty() {
                                password = (n < args.len()).then(|| args[n..].join(" "));
                                group = name;
                                break;
                            }
                        }
                        match found[..] {
                            [id] => id,
                            [] => return reply(&format!("Group {group} not found.")),
                            _ => {
                                return reply(&format!(
                                    "More than one group is called {group}: join it by its ID."
                                ))
                            }
                        }
                    }
                };
                match self.join(player, id, password) {
                    Ok(()) => {
                        let name = self.groups.iter().find(|g| g.id == id)?.name.clone();
                        reply(&format!("You joined the group {name}."))
                    }
                    Err(Join::NoGroup) => reply("That group doesn't exist."),
                    Err(Join::WrongPassword) => reply("Wrong password."),
                    Err(Join::NoMod) => None,
                }
            }
            ("leave", []) => {
                if self.voicers[&player].group.is_none() {
                    return reply("You are not in a group.");
                }
                self.leave(player);
                reply("You left the group.")
            }
            _ => Some(HELP.iter().map(|s| s.to_string()).collect()),
        }
    }
}
