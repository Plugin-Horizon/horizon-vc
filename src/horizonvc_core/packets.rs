//! The two halves of the protocol: plugin channel payloads (over the game connection) and
//! the packets inside the encrypted voice datagrams. The layouts are those the client mod
//! expects; they were worked out clean-room (see the README).
use crate::horizonvc_core::wire::{Reader, Result, Writer};

/// The protocol compatibility number of the client mod (2.6.x) and the only one a secret request is
/// answered for.
pub const COMPATIBILITY_VERSION: i32 = 20;

/// The channels a client sends on, in the order they are registered.
pub const INCOMING: [&str; 5] = [
    "voicechat:leave_group",
    "voicechat:request_secret",
    "voicechat:create_group",
    "voicechat:set_group",
    "voicechat:update_state",
];

/// `minecraft:register` with the incoming channels, each followed by a NUL.
pub fn register_payload() -> Vec<u8> {
    INCOMING.iter().flat_map(|c| c.bytes().chain([0])).collect()
}

/// What the server knows of one player, as `voicechat:state(s)` carry it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerState {
    pub disabled: bool,
    pub disconnected: bool,
    pub uuid: u128,
    pub name: String,
    pub group: Option<u128>,
}

impl PlayerState {
    fn write(&self, w: &mut Writer) {
        w.bool(self.disabled)
            .bool(self.disconnected)
            .uuid(self.uuid)
            .string(&self.name)
            .opt_uuid(self.group);
    }
}

pub fn state(s: &PlayerState) -> Vec<u8> {
    let mut w = Writer::default();
    s.write(&mut w);
    w.0
}

pub fn states(all: &[PlayerState]) -> Vec<u8> {
    let mut w = Writer::default();
    w.i32(all.len() as i32);
    for s in all {
        s.write(&mut w);
    }
    w.0
}

pub fn remove_state(uuid: u128) -> Vec<u8> {
    Writer::default().uuid(uuid).0.clone()
}

/// `voicechat:secret`: everything a client needs to open its voice connection.
pub struct Secret<'a> {
    pub secret: [u8; 16],
    pub port: i32,
    pub player: u128,
    pub codec: u8,
    pub mtu: i32,
    pub distance: f64,
    pub keep_alive: i32,
    pub groups: bool,
    pub voice_host: &'a str,
    pub allow_recording: bool,
}

pub fn secret(s: &Secret) -> Vec<u8> {
    let mut w = Writer::default();
    w.0.extend_from_slice(&s.secret);
    w.i32(s.port)
        .uuid(s.player)
        .u8(s.codec)
        .i32(s.mtu)
        .f64(s.distance)
        .i32(s.keep_alive)
        .bool(s.groups)
        .string(s.voice_host)
        .bool(s.allow_recording);
    w.0
}

/// A group as `voicechat:add_group` announces it. `kind`: 0 normal, 1 open, 2 isolated.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: u128,
    pub name: String,
    pub password: Option<String>,
    pub kind: i16,
    /// Kept when the last member leaves (made through the API).
    pub persistent: bool,
    /// Sent to the clients but not listed in their group menu (`Group.Builder.setHidden`).
    pub hidden: bool,
}

pub const NORMAL: i16 = 0;
pub const OPEN: i16 = 1;
pub const ISOLATED: i16 = 2;

pub fn add_group(g: &Group) -> Vec<u8> {
    let mut w = Writer::default();
    // Captured from a real server and client (tests/fixtures/captures): id, name, then the flags password,
    // persistent, hidden, then the type.
    w.uuid(g.id)
        .string(&g.name)
        .bool(g.password.is_some())
        .bool(g.persistent)
        .bool(g.hidden)
        .i16(g.kind);
    w.0
}

pub fn remove_group(id: u128) -> Vec<u8> {
    Writer::default().uuid(id).0.clone()
}

pub fn joined_group(group: Option<u128>, wrong_password: bool) -> Vec<u8> {
    let mut w = Writer::default();
    w.opt_uuid(group).bool(wrong_password);
    w.0
}

/// A payload a client sent on one of the `INCOMING` channels.
#[derive(Debug, PartialEq)]
pub enum FromClient {
    RequestSecret {
        version: i32,
    },
    UpdateState {
        disabled: bool,
    },
    CreateGroup {
        name: String,
        password: Option<String>,
        kind: i16,
    },
    SetGroup {
        id: u128,
        password: Option<String>,
    },
    LeaveGroup,
}

fn opt_string(r: &mut Reader, max: usize) -> Result<Option<String>> {
    Ok(if r.bool()? {
        Some(r.string(max)?)
    } else {
        None
    })
}

impl FromClient {
    pub fn decode(channel: &str, data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        Ok(match channel {
            "voicechat:request_secret" => Self::RequestSecret { version: r.i32()? },
            "voicechat:update_state" => Self::UpdateState {
                disabled: r.bool()?,
            },
            "voicechat:create_group" => Self::CreateGroup {
                name: r.string(512)?,
                password: opt_string(&mut r, 512)?,
                kind: r.i16()?,
            },
            "voicechat:set_group" => Self::SetGroup {
                id: r.uuid()?,
                password: opt_string(&mut r, 512)?,
            },
            "voicechat:leave_group" => Self::LeaveGroup,
            _ => return Err("not a voice chat channel"),
        })
    }
}

/// Packet ids inside a voice datagram.
pub mod id {
    pub const MIC: u8 = 1;
    pub const PLAYER_SOUND: u8 = 2;
    pub const GROUP_SOUND: u8 = 3;
    /// A locational audio channel's sound: the source position travels in the packet.
    pub const LOCATIONAL_SOUND: u8 = 4;
    pub const AUTHENTICATE: u8 = 5;
    pub const AUTHENTICATE_ACK: u8 = 6;
    pub const KEEP_ALIVE: u8 = 8;
    pub const CONNECTION_CHECK: u8 = 9;
    pub const CONNECTION_CHECK_ACK: u8 = 10;
}

/// A decrypted voice packet from a client.
#[derive(Debug, PartialEq)]
pub enum Voice<'a> {
    Mic {
        audio: &'a [u8],
        sequence: i64,
        whispering: bool,
    },
    Authenticate {
        player: u128,
        secret: [u8; 16],
    },
    KeepAlive,
    ConnectionCheck,
    Other(u8),
}

impl<'a> Voice<'a> {
    pub fn decode(plain: &'a [u8], mtu: usize) -> Result<Self> {
        let mut r = Reader::new(plain);
        Ok(match r.u8()? {
            id::MIC => Voice::Mic {
                audio: r.bytes(mtu)?,
                sequence: r.i64()?,
                whispering: r.bool()?,
            },
            id::AUTHENTICATE => Voice::Authenticate {
                player: r.uuid()?,
                secret: r.uuid()?.to_be_bytes(),
            },
            id::KEEP_ALIVE => Voice::KeepAlive,
            id::CONNECTION_CHECK => Voice::ConnectionCheck,
            other => Voice::Other(other),
        })
    }
}

/// A player's voice for one listener: proximity (`distance` set) or group sound.
pub fn sound(sender: u128, audio: &[u8], sequence: i64, proximity: Option<(f32, bool)>) -> Vec<u8> {
    sound_in(sender, audio, sequence, proximity, None)
}

/// The flag bit after the distance (or alone, in group sound) that says a volume category follows.
pub const SOUND_HAS_CATEGORY: u8 = 0b10;

/// [`sound`] with the volume category of the channel (`AudioChannel.setCategory`): the closing flags byte
/// (bit 0 whispering) gets [`SOUND_HAS_CATEGORY`] and the category id follows it as a string.
pub fn sound_in(
    sender: u128,
    audio: &[u8],
    sequence: i64,
    proximity: Option<(f32, bool)>,
    category: Option<&str>,
) -> Vec<u8> {
    let mut w = Writer::default();
    let pid = if proximity.is_some() {
        id::PLAYER_SOUND
    } else {
        id::GROUP_SOUND
    };
    // The channel id is the sender's UUID for both kinds.
    w.u8(pid).uuid(sender).uuid(sender).bytes(audio).i64(sequence);
    let mut flags = if category.is_some() { SOUND_HAS_CATEGORY } else { 0 };
    if let Some((distance, whispering)) = proximity {
        w.f32(distance);
        flags |= whispering as u8;
    }
    w.u8(flags);
    if let Some(category) = category {
        w.string(category);
    }
    w.0
}

/// The sound of a locational audio channel (`createLocationalAudioChannel`), as a real server sends it: the id, the
/// channel id twice, the source position (three f64), the audio, the sequence number, the distance (f32) and the
/// closing flags byte (bit 0b10: a category string follows).
pub fn locational_sound(
    channel: u128,
    pos: [f64; 3],
    audio: &[u8],
    sequence: i64,
    distance: f32,
    category: Option<&str>,
) -> Vec<u8> {
    let mut w = Writer::default();
    w.u8(id::LOCATIONAL_SOUND).uuid(channel).uuid(channel);
    w.f64(pos[0]).f64(pos[1]).f64(pos[2]);
    w.bytes(audio).i64(sequence).f32(distance);
    w.u8(if category.is_some() { SOUND_HAS_CATEGORY } else { 0 });
    if let Some(category) = category {
        w.string(category);
    }
    w.0
}

/// `voicechat:add_category`: the slider a client shows for a volume category. As captured from a real server: the id and
/// the name (strings), a flag byte (always 0 in the captures, meaning unknown), the description (a flag byte and then
/// the string), a second flag byte (0, unknown), and the icon (a flag byte and then 256 ARGB ints, row by row of the
/// `int[x][y]` the API takes).
pub fn add_category(id: &str, name: &str, description: &str, icon: Option<&[i32]>) -> Vec<u8> {
    let mut w = Writer::default();
    w.string(id).string(name).bool(false);
    if description.is_empty() {
        w.bool(false);
    } else {
        w.bool(true).string(description);
    }
    w.bool(false);
    match icon {
        Some(pixels) => {
            w.bool(true);
            for px in pixels {
                w.i32(*px);
            }
        }
        None => {
            w.bool(false);
        }
    }
    w.0
}

/// `voicechat:remove_category`.
pub fn remove_category(id: &str) -> Vec<u8> {
    let mut w = Writer::default();
    w.string(id);
    w.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    const A: u128 = 0x35deb110_752a_3928_95c4_cc89250ce08e;
    const B: u128 = 0x0d3214da_0cab_3a46_8ffd_470d2bef9d55;

    // The expected bytes below are the payload layouts the client mod reads.
    #[test]
    fn states_layout() {
        let a = PlayerState {
            disabled: false,
            disconnected: true,
            uuid: A,
            name: "ProbeA".into(),
            group: None,
        };
        assert_eq!(
            states(&[a]),
            hex("00000001000135deb110752a392895c4cc89250ce08e0650726f62654100")
        );
        let b = PlayerState {
            disabled: true,
            disconnected: false,
            uuid: B,
            name: "ProbeB".into(),
            group: None,
        };
        assert_eq!(
            state(&b),
            hex("01000d3214da0cab3a468ffd470d2bef9d550650726f62654200")
        );
        let g = PlayerState {
            group: Some(0x43c6f9a1_89b5_41aa_ae2d_aeb4af58cb80),
            disabled: false,
            disconnected: false,
            uuid: A,
            name: "ProbeA".into(),
        };
        assert_eq!(state(&g), hex("000035deb110752a392895c4cc89250ce08e0650726f6265410143c6f9a189b541aaae2daeb4af58cb80"));
        assert_eq!(remove_state(B), hex("0d3214da0cab3a468ffd470d2bef9d55"));
    }

    #[test]
    fn secret_layout() {
        let s = Secret {
            secret: hex("a4a73f4a11848c3feae62223224de295").try_into().unwrap(),
            port: 25666,
            player: A,
            codec: 0,
            mtu: 1275,
            distance: 48.0,
            keep_alive: 1000,
            groups: true,
            voice_host: "",
            allow_recording: true,
        };
        assert_eq!(
            secret(&s),
            hex("a4a73f4a11848c3feae62223224de2950000644235deb110752a392895c4cc89250ce08e00000004fb4048000000000000000003e8010001")
        );
    }

    #[test]
    fn groups_layout() {
        let g = Group {
            id: 0xcff60899_6009_467b_8a14_40518708bda1,
            name: "t1".into(),
            password: None,
            kind: OPEN,
            persistent: false,
            hidden: false,
        };
        assert_eq!(
            add_group(&g),
            hex("cff608996009467b8a1440518708bda10274310000000001")
        );
        let p = Group {
            id: 0xa8c344f3_3798_43de_9b83_03e77dc7e52d,
            name: "pw".into(),
            password: Some("x".into()),
            kind: NORMAL,
            persistent: false,
            hidden: false,
        };
        assert_eq!(
            add_group(&p),
            hex("a8c344f3379843de9b8303e77dc7e52d0270770100000000")
        );
        assert_eq!(joined_group(None, true), hex("0001"));
        assert_eq!(
            joined_group(Some(g.id), false),
            hex("01cff608996009467b8a1440518708bda100")
        );
    }

    #[test]
    fn client_payloads_decode() {
        assert_eq!(
            FromClient::decode("voicechat:request_secret", &hex("00000014")),
            Ok(FromClient::RequestSecret { version: 20 })
        );
        assert!(FromClient::decode("voicechat:request_secret", &[]).is_err());
        assert_eq!(
            FromClient::decode("voicechat:create_group", &hex("0270770106736573616d650000")),
            Ok(FromClient::CreateGroup {
                name: "pw".into(),
                password: Some("sesame".into()),
                kind: 0
            })
        );
        assert!(FromClient::decode("voicechat:create_group", &hex("02673500")).is_err());
        assert_eq!(
            FromClient::decode("voicechat:leave_group", &[]),
            Ok(FromClient::LeaveGroup)
        );
    }

    #[test]
    fn sounds_layout() {
        let audio: Vec<u8> = (0..40).collect();
        let mut want = vec![id::PLAYER_SOUND];
        want.extend(hex(
            "35deb110752a392895c4cc89250ce08e35deb110752a392895c4cc89250ce08e28",
        ));
        want.extend(&audio);
        want.extend(hex("000000000000000841c0000001"));
        assert_eq!(sound(A, &audio, 8, Some((24.0, true))), want);
        let mut group = vec![id::GROUP_SOUND];
        group.extend(hex("35deb110752a392895c4cc89250ce08e35deb110752a392895c4cc89250ce08e03010203000000000000000500"));
        assert_eq!(sound(A, &[1, 2, 3], 5, None), group);
    }

    // The category flag (0b10) is OR-ed into the closing flags byte and the id follows as a string.
    #[test]
    fn sounds_carry_their_volume_category() {
        let group = sound_in(A, &[1], 5, None, Some("radio"));
        let tail = &group[group.len() - 7..];
        assert_eq!(tail, [0x02, 0x05, b'r', b'a', b'd', b'i', b'o']);
        let near = sound_in(A, &[1], 5, Some((24.0, true)), Some("radio"));
        assert_eq!(&near[near.len() - 11..], [0x41, 0xc0, 0, 0, 0x03, 0x05, b'r', b'a', b'd', b'i', b'o']);
        assert_eq!(sound_in(A, &[1, 2, 3], 5, None, None), sound(A, &[1, 2, 3], 5, None));
    }

    #[test]
    fn category_packets() {
        let mut want = vec![5u8];
        want.extend(b"radio");
        want.push(5);
        want.extend(b"Radio");
        want.push(0); // the first unknown flag (captured as 0)
        want.push(1);
        want.push(30);
        want.extend(b"Adjust the lobby radio volume.");
        want.push(0); // the second unknown flag
        want.push(0); // no icon
        assert_eq!(add_category("radio", "Radio", "Adjust the lobby radio volume.", None), want);
        assert_eq!(remove_category("radio"), [5, b'r', b'a', b'd', b'i', b'o']);
    }

    #[test]
    fn mic_decodes_and_respects_the_mtu() {
        let mut p = vec![id::MIC, 3, 9, 9, 9];
        p.extend(7i64.to_be_bytes());
        p.push(1);
        assert_eq!(
            Voice::decode(&p, 1275),
            Ok(Voice::Mic {
                audio: &[9, 9, 9],
                sequence: 7,
                whispering: true
            })
        );
        assert!(Voice::decode(&p, 2).is_err());
    }

    #[test]
    fn register_lists_the_incoming_channels() {
        assert_eq!(
            register_payload(),
            b"voicechat:leave_group\0voicechat:request_secret\0voicechat:create_group\0voicechat:set_group\0voicechat:update_state\0"
        );
    }

    // Round trips: what a client would send, written with our own writer, decodes to the same
    // values, and our sound packets read back field by field.
    #[test]
    fn client_payloads_round_trip() {
        let mut w = Writer::default();
        w.i32(20);
        assert_eq!(
            FromClient::decode("voicechat:request_secret", &w.0),
            Ok(FromClient::RequestSecret { version: 20 })
        );
        let mut w = Writer::default();
        w.bool(true);
        assert_eq!(
            FromClient::decode("voicechat:update_state", &w.0),
            Ok(FromClient::UpdateState { disabled: true })
        );
        let mut w = Writer::default();
        w.string("Team").bool(true).string("pw").i16(2);
        assert_eq!(
            FromClient::decode("voicechat:create_group", &w.0),
            Ok(FromClient::CreateGroup {
                name: "Team".into(),
                password: Some("pw".into()),
                kind: 2
            })
        );
        let mut w = Writer::default();
        w.uuid(A).bool(false);
        assert_eq!(
            FromClient::decode("voicechat:set_group", &w.0),
            Ok(FromClient::SetGroup {
                id: A,
                password: None
            })
        );
        assert_eq!(
            FromClient::decode("voicechat:leave_group", &[]),
            Ok(FromClient::LeaveGroup)
        );
        assert!(FromClient::decode("voicechat:secret", &[]).is_err());
        assert!(FromClient::decode("voicechat:request_secret", &[0, 0]).is_err());
    }

    #[test]
    fn voice_packets_round_trip() {
        let mut w = Writer::default();
        w.u8(id::MIC).bytes(&[1, 2, 3]).i64(77).bool(false);
        assert_eq!(
            Voice::decode(&w.0, 1024),
            Ok(Voice::Mic {
                audio: &[1, 2, 3],
                sequence: 77,
                whispering: false
            })
        );
        let mut w = Writer::default();
        w.u8(id::AUTHENTICATE).uuid(A).uuid(B);
        assert_eq!(
            Voice::decode(&w.0, 1024),
            Ok(Voice::Authenticate {
                player: A,
                secret: B.to_be_bytes()
            })
        );
        assert_eq!(Voice::decode(&[id::KEEP_ALIVE], 1024), Ok(Voice::KeepAlive));
        assert_eq!(
            Voice::decode(&[id::CONNECTION_CHECK], 1024),
            Ok(Voice::ConnectionCheck)
        );
        assert_eq!(Voice::decode(&[42], 1024), Ok(Voice::Other(42)));
        assert!(Voice::decode(&[], 1024).is_err());
    }

    #[test]
    fn sound_packets_read_back() {
        let proximity = sound(A, &[9, 8], 5, Some((48.0, true)));
        let mut r = Reader::new(&proximity);
        assert_eq!(r.u8(), Ok(id::PLAYER_SOUND));
        assert_eq!(r.uuid(), Ok(A));
        assert_eq!(r.uuid(), Ok(A));
        assert_eq!(r.bytes(1024), Ok(&[9u8, 8][..]));
        assert_eq!(r.i64(), Ok(5));
        assert_eq!(r.i32().map(|b| f32::from_bits(b as u32)), Ok(48.0));
        assert_eq!(r.bool(), Ok(true));
        assert!(r.rest().is_empty());
        let group = sound(B, &[1], -1, None);
        let mut r = Reader::new(&group);
        assert_eq!(r.u8(), Ok(id::GROUP_SOUND));
        assert_eq!(r.uuid(), Ok(B));
        assert_eq!(r.uuid(), Ok(B));
        assert_eq!(r.bytes(1024), Ok(&[1u8][..]));
        assert_eq!(r.i64(), Ok(-1));
        assert_eq!(r.u8(), Ok(0));
        assert!(r.rest().is_empty());
    }

    #[test]
    fn sealed_datagram_round_trips() {
        let secret = [7u8; 16];
        let mut w = Writer::default();
        w.u8(id::MIC).bytes(&[4, 5]).i64(1).bool(true);
        let sealed = crate::horizonvc_core::crypto::seal(&secret, &w.0);
        let plain = crate::horizonvc_core::crypto::open(&secret, &sealed).expect("opens with its secret");
        assert_eq!(
            Voice::decode(&plain, 1024),
            Ok(Voice::Mic {
                audio: &[4, 5],
                sequence: 1,
                whispering: true
            })
        );
        assert!(crate::horizonvc_core::crypto::open(&[8u8; 16], &sealed).is_none());
    }
}
