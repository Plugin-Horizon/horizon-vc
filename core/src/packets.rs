//! The two halves of the protocol: plugin channel payloads (over the game connection) and
//! the packets inside the encrypted voice datagrams. The layouts are those the client mod
//! expects; they were worked out clean-room (see the README).
use crate::wire::{Reader, Result, Writer};

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
}

pub const NORMAL: i16 = 0;
pub const OPEN: i16 = 1;
pub const ISOLATED: i16 = 2;

pub fn add_group(g: &Group) -> Vec<u8> {
    let mut w = Writer::default();
    // The two flags after the password one are always false (HorizonVC has no hidden or
    // config-made groups).
    w.uuid(g.id)
        .string(&g.name)
        .bool(g.password.is_some())
        .bool(false)
        .bool(false)
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
    let mut w = Writer::default();
    let pid = if proximity.is_some() {
        id::PLAYER_SOUND
    } else {
        id::GROUP_SOUND
    };
    // The channel id is the sender's UUID for both kinds.
    w.u8(pid)
        .uuid(sender)
        .uuid(sender)
        .bytes(audio)
        .i64(sequence);
    match proximity {
        Some((distance, whispering)) => w.f32(distance).bool(whispering),
        None => w.u8(0),
    };
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
        let sealed = crate::crypto::seal(&secret, &w.0);
        let plain = crate::crypto::open(&secret, &sealed).expect("opens with its secret");
        assert_eq!(
            Voice::decode(&plain, 1024),
            Ok(Voice::Mic {
                audio: &[4, 5],
                sequence: 1,
                whispering: true
            })
        );
        assert!(crate::crypto::open(&[8u8; 16], &sealed).is_none());
    }
}
