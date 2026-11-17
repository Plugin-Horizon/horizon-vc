//! `horizonvc-server.properties`: HorizonVC's server config, a Java-style key=value file. Only the
//! keys listed in the README are acted on; the
//! others are read and ignored. A missing file is written with the defaults.
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// UDP port; -1 uses the game's `server-port`.
    pub port: i32,
    /// Empty uses `server-ip` from server.properties, `*` binds every address.
    pub bind_address: String,
    pub max_voice_distance: f64,
    pub whisper_distance: f64,
    /// `VOIP`, `AUDIO` or `RESTRICTED_LOWDELAY`: passed to the clients only.
    pub codec: String,
    pub mtu_size: i32,
    /// Plugin channel messages a player may send a second; more are dropped. -1 disables.
    pub tcp_rate_limit: i32,
    /// Milliseconds between keep-alives; ten missed ones disconnect the voice.
    pub keep_alive: i32,
    pub enable_groups: bool,
    /// Host (and port) the clients connect their voice to; empty = the game's host.
    pub voice_host: String,
    pub allow_recording: bool,
    /// Below 0: `max_voice_distance`.
    pub broadcast_range: f64,
    /// Kick players whose client did not ask for a voice secret within `login_timeout` ms.
    pub force_voice_chat: bool,
    pub login_timeout: i32,
    /// Read but not acted on (see the README, "Not supported yet").
    pub spectator_interaction: bool,
    pub spectator_player_possession: bool,
    pub allow_pings: bool,
    pub use_natives: bool,
    pub threaded_server_support: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 24454,
            bind_address: String::new(),
            max_voice_distance: 48.0,
            whisper_distance: 24.0,
            codec: "VOIP".into(),
            mtu_size: 1275,
            tcp_rate_limit: 16,
            keep_alive: 1000,
            enable_groups: true,
            voice_host: String::new(),
            allow_recording: true,
            broadcast_range: -1.0,
            force_voice_chat: false,
            login_timeout: 10_000,
            spectator_interaction: false,
            spectator_player_possession: false,
            allow_pings: true,
            use_natives: true,
            threaded_server_support: true,
        }
    }
}

pub const DEFAULT_FILE: &str = include_str!("../horizonvc-server.properties");

fn number<T: std::str::FromStr>(line: usize, key: &str, value: &str, what: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("line {line}: {key} is not {what}"))
}

fn flag(line: usize, key: &str, value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("line {line}: {key} is not true or false")),
    }
}

impl Config {
    /// Reads Java-style `key=value` lines: `#` and `!` start a comment line, blank lines are skipped.
    /// Unknown keys are ignored; a bad value is an error (the server then uses the defaults).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut c = Config::default();
        for (i, raw) in text.lines().enumerate() {
            let n = i + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {n}: no '=' in {line:?}"))?;
            let (key, value) = (key.trim(), value.trim());
            match key {
                "port" => c.port = number(n, key, value, "an integer")?,
                "bind_address" => c.bind_address = value.to_owned(),
                "max_voice_distance" => c.max_voice_distance = number(n, key, value, "a number")?,
                "whisper_distance" => c.whisper_distance = number(n, key, value, "a number")?,
                "codec" => c.codec = value.to_owned(),
                "mtu_size" => c.mtu_size = number(n, key, value, "an integer")?,
                "tcp_rate_limit" => c.tcp_rate_limit = number(n, key, value, "an integer")?,
                "keep_alive" => c.keep_alive = number::<i32>(n, key, value, "an integer")?.max(1),
                "enable_groups" => c.enable_groups = flag(n, key, value)?,
                "voice_host" => c.voice_host = value.to_owned(),
                "allow_recording" => c.allow_recording = flag(n, key, value)?,
                "broadcast_range" => c.broadcast_range = number(n, key, value, "a number")?,
                "force_voice_chat" => c.force_voice_chat = flag(n, key, value)?,
                "login_timeout" => c.login_timeout = number::<i32>(n, key, value, "an integer")?.max(0),
                "spectator_interaction" => c.spectator_interaction = flag(n, key, value)?,
                "spectator_player_possession" => c.spectator_player_possession = flag(n, key, value)?,
                "allow_pings" => c.allow_pings = flag(n, key, value)?,
                "use_natives" => c.use_natives = flag(n, key, value)?,
                "threaded_server_support" => c.threaded_server_support = flag(n, key, value)?,
                _ => {}
            }
        }
        Ok(c)
    }

    /// Reads `path`, writing the defaults there first when it is missing.
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            std::fs::write(path, DEFAULT_FILE).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text)
    }

    /// The codec byte of `voicechat:secret` (`VOIP` = 0, `AUDIO` = 1,
    /// `RESTRICTED_LOWDELAY` = 2).
    pub fn codec_id(&self) -> u8 {
        match self.codec.as_str() {
            "AUDIO" => 1,
            "RESTRICTED_LOWDELAY" => 2,
            _ => 0,
        }
    }

    /// Players farther than this hear nothing of a proximity voice. With a range of 48 a voice
    /// is delivered at 49.0 blocks and not at 49.25.
    pub fn cutoff(&self) -> f64 {
        let range = if self.broadcast_range < 0.0 {
            self.max_voice_distance
        } else {
            self.broadcast_range
        };
        range + 1.0
    }
}

/// `key=value` from a server.properties file.
pub fn server_property(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .find_map(|l| {
            l.split_once('=')
                .filter(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim().to_owned())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_file_parses_to_the_defaults() {
        assert_eq!(Config::parse(DEFAULT_FILE), Ok(Config::default()));
    }

    #[test]
    fn values_override_the_defaults() {
        let c = Config::parse("port=25666\nmax_voice_distance=32\nenable_groups=false\n").unwrap();
        assert_eq!(
            (c.port, c.max_voice_distance, c.enable_groups),
            (25666, 32.0, false)
        );
        assert_eq!(c.cutoff(), 33.0);
        assert!(Config::parse("port=abc\n").is_err());
        assert!(Config::parse("enable_groups=yes\n").is_err());
        let f = Config::parse("force_voice_chat=true\nlogin_timeout=5000\nallow_pings=false\n").unwrap();
        assert_eq!(
            (f.force_voice_chat, f.login_timeout, f.allow_pings),
            (true, 5000, false)
        );
    }

    #[test]
    fn server_properties_are_read() {
        let p = "#x\nserver-ip=127.0.0.1\nserver-port = 25665\n";
        assert_eq!(
            server_property(p, "server-ip").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(server_property(p, "server-port").as_deref(), Some("25665"));
        assert_eq!(server_property(p, "motd"), None);
    }
}
