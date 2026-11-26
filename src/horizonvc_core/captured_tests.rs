//! Wire captures of the original Simple Voice Chat server (Paper plugin 2.6.27, Velocity plugin 2.6.26) with the
//! original Fabric client (2.6.27+26.3, Minecraft 26.3), recorded on 2026-11-25 with realclient/vcbench/vc_capture.py
//! (see CLEANROOM.md, "Capture round 2026-11-25"). Every payload the port builds must be byte-identical to a
//! captured one, and every payload a client sent must decode. The files hold one `{dir, chan, hex}` per plugin message
//! (`s2c`: server to client, `c2s`: client to server); `sounds_plain.txt` holds decrypted voice packets.
use super::packets::{self as p, FromClient, Group, Secret};
use super::wire::Reader;

const RUN1: &str = include_str!("../../tests/fixtures/captures/run1_tcp.jsonl");
const PROXY: &str = include_str!("../../tests/fixtures/captures/proxy1_tcp.jsonl");
const BACKEND: &str = include_str!("../../tests/fixtures/captures/backend_side_tcp.jsonl");
const ICON: &str = include_str!("../../tests/fixtures/captures/icon_tcp.jsonl");
const SOUNDS: &str = include_str!("../../tests/fixtures/captures/sounds_plain.txt");

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// (direction, channel, payload) of every captured message.
fn rows(text: &str) -> Vec<(String, String, Vec<u8>)> {
    text.lines()
        .map(|l| {
            let field = |key: &str| {
                let at = l.find(&format!("\"{key}\": \"")).unwrap() + key.len() + 5;
                l[at..].split('"').next().unwrap().to_owned()
            };
            (field("dir"), field("chan"), hex(&field("hex")))
        })
        .collect()
}

fn of<'a>(all: &'a [(String, String, Vec<u8>)], dir: &str, chan: &str) -> Vec<&'a Vec<u8>> {
    all.iter().filter(|r| r.0 == dir && r.1 == chan).map(|r| &r.2).collect()
}

fn group_of(data: &[u8]) -> Group {
    let mut r = Reader::new(data);
    let g = Group {
        id: r.uuid().unwrap(),
        name: r.string(512).unwrap(),
        password: r.bool().unwrap().then(String::new),
        persistent: r.bool().unwrap(),
        hidden: r.bool().unwrap(),
        kind: 0,
    };
    let kind = r.i16().unwrap();
    assert!(r.rest().is_empty());
    Group { kind, ..g }
}

#[test]
fn every_captured_add_group_is_rebuilt_byte_for_byte() {
    let all = rows(RUN1);
    let groups = of(&all, "s2c", "voicechat:add_group");
    // API groups on three joins and at runtime, then the 4 groups the client made in its menu.
    assert!(groups.len() >= 10, "{} captured", groups.len());
    let mut flags = std::collections::BTreeSet::new();
    for data in groups {
        let g = group_of(data);
        let rebuilt = Group { password: g.password.as_ref().map(|_| "x".into()), ..g.clone() };
        assert_eq!(&p::add_group(&rebuilt), data, "{}", g.name);
        flags.insert((g.password.is_some(), g.persistent, g.hidden, g.kind));
    }
    // Every flag the original uses: password, persistent (API), hidden (API), and all three types.
    assert!(flags.contains(&(true, true, true, p::ISOLATED)), "hidden + password + persistent + isolated");
    assert!(flags.contains(&(false, true, true, p::NORMAL)), "hidden + persistent");
    assert!(flags.contains(&(true, false, false, p::NORMAL)), "client-made, with a password");
    assert!(flags.contains(&(false, false, false, p::OPEN)));
    assert!(flags.contains(&(false, false, false, p::ISOLATED)));
}

#[test]
fn a_group_made_in_the_menu_is_never_persistent_or_hidden() {
    let all = rows(RUN1);
    let made: Vec<Group> = of(&all, "s2c", "voicechat:add_group")
        .into_iter()
        .map(|d| group_of(d))
        .filter(|g| g.name.starts_with("ui"))
        .collect();
    assert_eq!(made.len(), 4, "normal, with password, open, isolated");
    assert!(made.iter().all(|g| !g.persistent && !g.hidden));
}

#[test]
fn captured_client_payloads_decode() {
    let all = rows(RUN1);
    let creates: Vec<FromClient> = of(&all, "c2s", "voicechat:create_group")
        .into_iter()
        .map(|d| FromClient::decode("voicechat:create_group", d).unwrap())
        .collect();
    let want = |name: &str, password: Option<&str>, kind| FromClient::CreateGroup {
        name: name.into(),
        password: password.map(str::to_owned),
        kind,
    };
    assert_eq!(
        creates,
        vec![
            want("uiNormal", None, p::NORMAL),
            want("uiPw", Some("pw2"), p::NORMAL),
            want("uiOpen", None, p::OPEN),
            want("uiIso", None, p::ISOLATED),
        ]
    );
    let sets: Vec<FromClient> = of(&all, "c2s", "voicechat:set_group")
        .into_iter()
        .map(|d| FromClient::decode("voicechat:set_group", d).unwrap())
        .collect();
    assert!(matches!(&sets[0], FromClient::SetGroup { password: None, .. }), "open group, no password");
    assert!(matches!(&sets[1], FromClient::SetGroup { password: Some(pw), .. } if pw == "wrongpw"));
    assert!(matches!(&sets[2], FromClient::SetGroup { password: Some(pw), .. } if pw == "secret1"));
    for d in of(&all, "c2s", "voicechat:leave_group") {
        assert_eq!(FromClient::decode("voicechat:leave_group", d), Ok(FromClient::LeaveGroup));
    }
    assert_eq!(
        FromClient::decode("voicechat:request_secret", of(&all, "c2s", "voicechat:request_secret")[0]),
        Ok(FromClient::RequestSecret { version: 20 })
    );
}

#[test]
fn captured_joined_group_and_removals_are_rebuilt() {
    let all = rows(RUN1);
    let joined = of(&all, "s2c", "voicechat:joined_group");
    // Joined an open group, left it, wrong password, joined with the password.
    for data in &joined {
        let mut r = Reader::new(data);
        let group = r.bool().unwrap().then(|| r.uuid().unwrap());
        let wrong = r.bool().unwrap();
        assert_eq!(&&p::joined_group(group, wrong), data);
    }
    assert!(joined.iter().any(|d| d.as_slice() == [0, 1]), "the wrong-password answer is `00 01`");
    for data in of(&all, "s2c", "voicechat:remove_group") {
        assert_eq!(&p::remove_group(Reader::new(data).uuid().unwrap()), data);
    }
    for data in of(&all, "s2c", "voicechat:remove_category") {
        assert_eq!(&p::remove_category(&Reader::new(data).string(64).unwrap()), data);
    }
}

#[test]
fn captured_categories_are_rebuilt() {
    let all = [rows(RUN1), rows(ICON)].concat();
    let cats = of(&all, "s2c", "voicechat:add_category");
    // "radio" on each of three joins, "music" once at runtime; then one with an icon and one without a description.
    assert_eq!(cats.len(), 6);
    let mut icons = 0;
    for data in cats {
        let mut r = Reader::new(data);
        let (id, name) = (r.string(64).unwrap(), r.string(64).unwrap());
        assert!(!r.bool().unwrap(), "first unknown flag");
        let description = if r.bool().unwrap() { r.string(256).unwrap() } else { String::new() };
        assert!(!r.bool().unwrap(), "second unknown flag");
        let icon: Option<Vec<i32>> = r.bool().unwrap().then(|| (0..256).map(|_| r.i32().unwrap()).collect());
        assert!(r.rest().is_empty());
        icons += icon.is_some() as usize;
        assert_eq!(&p::add_category(&id, &name, &description, icon.as_deref()), data, "{id}");
    }
    assert_eq!(icons, 1);
    assert_eq!(of(&all, "s2c", "voicechat:add_category")[0], &hex("05726164696f05526164696f000109526164696f446573630000"));
}

/// The order of the messages after a secret request, as the original sends them: states, categories, groups, secret.
#[test]
fn the_join_burst_is_states_categories_groups_secret() {
    let all = rows(RUN1);
    let burst: Vec<&str> = all.iter().filter(|r| r.0 == "s2c").map(|r| r.1.as_str()).take(9).collect();
    assert_eq!(
        burst,
        [
            "voicechat:states",
            "voicechat:add_category",
            "voicechat:add_group",
            "voicechat:add_group",
            "voicechat:add_group",
            "voicechat:add_group",
            "voicechat:add_group",
            "voicechat:add_group",
            "voicechat:secret"
        ]
    );
}

fn secret_of(data: &[u8]) -> (Vec<u8>, i32, u128, String) {
    let mut r = Reader::new(data);
    let key = (0..16).map(|_| r.u8().unwrap()).collect::<Vec<_>>();
    let port = r.i32().unwrap();
    let player = r.uuid().unwrap();
    let (codec, mtu) = (r.u8().unwrap(), r.i32().unwrap());
    assert_eq!((codec, mtu), (0, 1275));
    let _distance = (0..8).map(|_| r.u8().unwrap()).collect::<Vec<_>>();
    assert_eq!(r.i32().unwrap(), 1000);
    assert!(r.bool().unwrap());
    let host = r.string(255).unwrap();
    assert!(r.bool().unwrap());
    assert!(r.rest().is_empty());
    (key, port, player, host)
}

/// Item 4, the proxy: the secret the backend sent (seen at the backend) reaches the client with its key and player
/// unchanged; the proxy only replaces the port and the voice host with its own. So a backend needs nothing beyond
/// the ordinary secret packet, with its real voice port in it.
#[test]
fn behind_the_proxy_only_the_port_and_the_voice_host_of_the_secret_change() {
    let backend = rows(BACKEND);
    let client = rows(PROXY);
    let at_backend = secret_of(of(&backend, "s2c", "voicechat:secret")[0]);
    let secrets = of(&client, "s2c", "voicechat:secret");
    // The second session of the proxy capture is the one that went through the tee that recorded the backend side.
    let at_client = secret_of(secrets[1]);
    assert_eq!(at_backend.0, at_client.0, "same key");
    assert_eq!(at_backend.2, at_client.2, "same player");
    assert_eq!((at_backend.1, at_backend.3.as_str()), (24460, "127.0.0.1:24461"), "the backend's own port and host");
    assert_eq!((at_client.1, at_client.3.as_str()), (25622, "127.0.0.1:25623"), "the proxy's UDP port and voice_host");
    // The port's secret writer produces the backend's bytes (direct and behind the proxy alike).
    let rebuilt = p::secret(&Secret {
        secret: at_backend.0.clone().try_into().unwrap(),
        port: 24460,
        player: at_backend.2,
        codec: 0,
        mtu: 1275,
        distance: 48.0,
        keep_alive: 1000,
        groups: true,
        voice_host: "127.0.0.1:24461",
        allow_recording: true,
    });
    assert_eq!(&rebuilt, of(&backend, "s2c", "voicechat:secret")[0]);
}

#[test]
fn captured_sound_packets_are_rebuilt() {
    let mut kinds = std::collections::BTreeSet::new();
    for line in SOUNDS.lines() {
        let plain = hex(line.split_whitespace().nth(3).unwrap());
        let mut r = Reader::new(&plain);
        let id = r.u8().unwrap();
        let channel = r.uuid().unwrap();
        assert_eq!(r.uuid().unwrap(), channel, "the channel id is in both uuid places");
        let pos = (id == p::id::LOCATIONAL_SOUND).then(|| {
            let mut v = [0f64; 3];
            for x in &mut v {
                *x = f64::from_be_bytes((0..8).map(|_| r.u8().unwrap()).collect::<Vec<_>>().try_into().unwrap());
            }
            v
        });
        let audio = r.bytes(2048).unwrap().to_vec();
        let sequence = r.i64().unwrap();
        let distance = pos.map(|_| f32::from_be_bytes((0..4).map(|_| r.u8().unwrap()).collect::<Vec<_>>().try_into().unwrap()));
        let flags = r.u8().unwrap();
        let category = (flags & p::SOUND_HAS_CATEGORY != 0).then(|| r.string(64).unwrap());
        assert!(r.rest().is_empty());
        let rebuilt = match pos {
            Some(pos) => p::locational_sound(channel, pos, &audio, sequence, distance.unwrap(), category.as_deref()),
            None => p::sound_in(channel, &audio, sequence, None, category.as_deref()),
        };
        assert_eq!(rebuilt, plain, "id {id} seq {sequence}");
        kinds.insert((id, category));
    }
    assert_eq!(kinds.len(), 4, "static and locational, with the radio category and without");
}
