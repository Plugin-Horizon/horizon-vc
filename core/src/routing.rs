//! Who hears a player's voice, as the client mod expects it:
//! proximity in the same dimension up to `Config::cutoff`, no special case for spectators,
//! disabled players hear nothing (but are heard), and groups by their kind:
//! - every other member of the speaker's group gets a group sound, at any distance;
//! - outsiders in range hear a member only of an open group;
//! - members of an isolated group hear no outsider; other members do.
use crate::packets::{ISOLATED, OPEN};

/// A voice-connected player as routing sees them.
#[derive(Clone, Debug)]
pub struct Listener<'a> {
    pub uuid: u128,
    pub dimension: &'a str,
    pub pos: [f64; 3],
    pub disabled: bool,
    pub group: Option<(u128, i16)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Delivery {
    Proximity,
    Group,
}

pub fn recipients(speaker: &Listener, everyone: &[Listener], cutoff: f64) -> Vec<(u128, Delivery)> {
    let near = |l: &Listener| {
        l.dimension == speaker.dimension && {
            let d: f64 = (0..3).map(|i| (l.pos[i] - speaker.pos[i]).powi(2)).sum();
            d <= cutoff * cutoff
        }
    };
    everyone
        .iter()
        .filter(|l| l.uuid != speaker.uuid && !l.disabled)
        .filter_map(|l| {
            let same_group =
                matches!((speaker.group, l.group), (Some((a, _)), Some((b, _))) if a == b);
            if same_group {
                return Some((l.uuid, Delivery::Group));
            }
            let speaker_audible = match speaker.group {
                Some((_, kind)) => kind == OPEN,
                None => true,
            };
            let listener_open = !matches!(l.group, Some((_, ISOLATED)));
            (speaker_audible && listener_open && near(l)).then_some((l.uuid, Delivery::Proximity))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::NORMAL;

    fn at(uuid: u128, x: f64) -> Listener<'static> {
        Listener {
            uuid,
            dimension: "minecraft:overworld",
            pos: [x, -60.0, 0.5],
            disabled: false,
            group: None,
        }
    }
    fn heard(speaker: &Listener, all: &[Listener]) -> Vec<(u128, Delivery)> {
        recipients(speaker, all, 49.0)
    }

    // The cases below fix the routing rules.
    #[test]
    fn proximity_cut_off() {
        let a = at(1, 0.5);
        for (x, want) in [(49.5, true), (49.75, false), (49.4, true), (50.5, false)] {
            let b = at(2, x);
            assert_eq!(!heard(&a, &[a.clone(), b]).is_empty(), want, "B at {x}");
        }
        let mut up = at(2, 0.5);
        up.pos[1] = -11.0;
        assert_eq!(heard(&a, &[up.clone()]).len(), 1);
        up.pos[1] = -10.5;
        assert!(heard(&a, &[up]).is_empty());
    }

    #[test]
    fn other_dimensions_and_disabled_players_hear_nothing() {
        let a = at(1, 0.5);
        let mut b = at(2, 0.5);
        b.dimension = "minecraft:the_nether";
        assert!(heard(&a, &[b]).is_empty());
        let mut c = at(3, 0.5);
        c.disabled = true;
        assert!(heard(&a, &[c.clone()]).is_empty());
        // A disabled speaker is still heard.
        assert_eq!(heard(&c, &[a]), vec![(1, Delivery::Proximity)]);
    }

    #[test]
    fn groups_by_kind() {
        for (kind, outsider_hears_member, member_hears_outsider) in [
            (NORMAL, false, true),
            (OPEN, true, true),
            (ISOLATED, false, false),
        ] {
            let g = Some((9, kind));
            let a = Listener {
                group: g,
                ..at(1, 0.5)
            };
            let b = Listener {
                group: g,
                ..at(2, 2.5)
            };
            let c = at(3, 4.5);
            let all = [a.clone(), b.clone(), c.clone()];
            let mut want = vec![(2, Delivery::Group)];
            if outsider_hears_member {
                want.push((3, Delivery::Proximity));
            }
            assert_eq!(heard(&a, &all), want, "kind {kind}: A speaks");
            let want_c: Vec<_> = if member_hears_outsider {
                vec![(1, Delivery::Proximity), (2, Delivery::Proximity)]
            } else {
                vec![]
            };
            assert_eq!(heard(&c, &all), want_c, "kind {kind}: C speaks");
            // Far away, members still hear each other.
            let far = Listener {
                pos: [200.5, -60.0, 0.5],
                ..b
            };
            assert_eq!(
                heard(&a, &[a.clone(), far.clone()]),
                vec![(2, Delivery::Group)]
            );
            assert_eq!(
                heard(&far, &[a.clone(), far.clone()]),
                vec![(1, Delivery::Group)]
            );
        }
    }
}
