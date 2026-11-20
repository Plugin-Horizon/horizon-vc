//! The player-visible texts of `plugins/voicechat/translations.properties`, with the English defaults the
//! original ships with when the file is missing. Only the four keys the port shows are read; the
//! Placeholder API keys and `player_command_message` have no use here.
use crate::horizonvc_core::properties;
use std::path::Path;

/// Name and version the original's `%s` placeholders are filled with.
pub const MOD_NAME: &str = "Simple Voice Chat";
pub const MOD_VERSION: &str = "2.6.27";

const DEFAULT_FORCE_KICK: &str = "You need %s %s to play on this server";
const DEFAULT_RATE_LIMIT_KICK: &str = "Kicked for exceeding packet rate limit";
const DEFAULT_NOT_COMPATIBLE: &str = "Your voice chat client version is not compatible with the server-side version.\nPlease install version %s of %s.";
const DEFAULT_COMMAND_NEEDED: &str = "You need to have %s installed on your client to use this command";

#[derive(Clone, Debug, PartialEq)]
pub struct Texts {
    /// Kick reason for a player without the mod under `force_voice_chat`. Placeholders: name, version.
    pub force_voicechat_kick_message: String,
    /// Kick reason after too many voice packets. No placeholders.
    pub rate_limit_kick_message: String,
    /// Chat line for a client with another protocol version. Placeholders: version, name.
    pub voicechat_not_compatible_message: String,
    /// Answer to a command that needs the mod on the client. Placeholder: name.
    pub voicechat_needed_for_command_message: String,
}

impl Default for Texts {
    fn default() -> Self {
        Self {
            force_voicechat_kick_message: DEFAULT_FORCE_KICK.into(),
            rate_limit_kick_message: DEFAULT_RATE_LIMIT_KICK.into(),
            voicechat_not_compatible_message: DEFAULT_NOT_COMPATIBLE.into(),
            voicechat_needed_for_command_message: DEFAULT_COMMAND_NEEDED.into(),
        }
    }
}

/// Fills the `%s` placeholders of `template` from `args`, in order. Extra placeholders stay as they are
/// and extra arguments are dropped, as far as the original's messages go.
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    let mut args = args.iter();
    while let Some(at) = rest.find("%s") {
        out.push_str(&rest[..at]);
        match args.next() {
            Some(a) => out.push_str(a),
            None => out.push_str("%s"),
        }
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

/// A value as the original shows it: Java escapes resolved, then the literal two characters `\n` (which
/// the file writes as `\\n`) become a line break.
fn shown(value: &str) -> String {
    value.replace("\\n", "\n")
}

impl Texts {
    /// Reads `translations.properties` text. A key that is missing or empty keeps its default.
    pub fn parse(text: &str) -> Self {
        let mut t = Texts::default();
        for (key, value) in properties::parse(text) {
            let value = shown(&value);
            if value.is_empty() {
                continue;
            }
            match key.as_str() {
                "force_voicechat_kick_message" => t.force_voicechat_kick_message = value,
                "rate_limit_kick_message" => t.rate_limit_kick_message = value,
                "voicechat_not_compatible_message" => t.voicechat_not_compatible_message = value,
                "voicechat_needed_for_command_message" => t.voicechat_needed_for_command_message = value,
                _ => {}
            }
        }
        t
    }

    /// Reads `path`; the defaults when it is missing or unreadable.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(_) => Self::default(),
        }
    }

    pub fn force_kick(&self) -> String {
        fill(&self.force_voicechat_kick_message, &[MOD_NAME, MOD_VERSION])
    }

    pub fn rate_limit_kick(&self) -> String {
        self.rate_limit_kick_message.clone()
    }

    pub fn incompatible(&self) -> String {
        fill(&self.voicechat_not_compatible_message, &[MOD_VERSION, MOD_NAME])
    }

    pub fn command_needed(&self) -> String {
        fill(&self.voicechat_needed_for_command_message, &[MOD_NAME])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_replaces_placeholders_in_order() {
        assert_eq!(fill("You need %s %s to play", &["A", "1.0"]), "You need A 1.0 to play");
        assert_eq!(fill("only %s", &[]), "only %s");
        assert_eq!(fill("no placeholder", &["x"]), "no placeholder");
        assert_eq!(fill("%s%s", &["a", "b"]), "ab");
    }

    #[test]
    fn the_defaults_read_back_as_the_original_shows_them() {
        assert_eq!(Texts::parse(""), Texts::default());
        assert_eq!(
            Texts::default().force_kick(),
            "You need Simple Voice Chat 2.6.27 to play on this server"
        );
        assert_eq!(Texts::default().rate_limit_kick(), "Kicked for exceeding packet rate limit");
        assert_eq!(
            Texts::default().incompatible(),
            "Your voice chat client version is not compatible with the server-side version.\nPlease install version 2.6.27 of Simple Voice Chat."
        );
        assert_eq!(
            Texts::default().command_needed(),
            "You need to have Simple Voice Chat installed on your client to use this command"
        );
    }

    #[test]
    fn the_file_overrides_the_defaults_and_the_escapes_are_applied() {
        let file = "# comment\n\
            force_voicechat_kick_message=Install %s %s first\n\
            rate_limit_kick_message=Too fast\n\
            voicechat_not_compatible_message=Old client.\\\\nGet version %s of %s.\n\
            voicechat_needed_for_command_message=Needs %s\n\
            player_command_message=unused\n";
        let t = Texts::parse(file);
        assert_eq!(t.force_kick(), "Install Simple Voice Chat 2.6.27 first");
        assert_eq!(t.rate_limit_kick(), "Too fast");
        assert_eq!(t.incompatible(), "Old client.\nGet version 2.6.27 of Simple Voice Chat.");
        assert_eq!(t.command_needed(), "Needs Simple Voice Chat");
    }

    #[test]
    fn an_empty_value_keeps_the_default() {
        let t = Texts::parse("rate_limit_kick_message=\n");
        assert_eq!(t, Texts::default());
    }
}
