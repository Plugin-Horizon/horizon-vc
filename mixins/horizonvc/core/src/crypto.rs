//! The voice datagrams' encryption, AES-128-GCM keyed with the player's
//! 16-byte secret, a fresh random 12-byte nonce in front of ciphertext and tag, no AAD.
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_128_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

pub fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    SystemRandom::new()
        .fill(&mut out)
        .expect("the system random source failed");
    out
}

fn key(secret: &[u8; 16]) -> LessSafeKey {
    LessSafeKey::new(UnboundKey::new(&AES_128_GCM, secret).expect("a 16-byte AES key"))
}

/// Nonce, then the ciphertext with its tag.
pub fn seal(secret: &[u8; 16], plain: &[u8]) -> Vec<u8> {
    let nonce: [u8; NONCE_LEN] = random();
    let mut data = plain.to_vec();
    key(secret)
        .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut data)
        .expect("sealing fits in memory");
    let mut out = nonce.to_vec();
    out.extend(data);
    out
}

/// The plaintext of `seal`'s output, or None when it was not sealed with this secret.
pub fn open(secret: &[u8; 16], sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < NONCE_LEN {
        return None;
    }
    let (nonce, body) = sealed.split_at(NONCE_LEN);
    let mut data = body.to_vec();
    let nonce = Nonce::try_assume_unique_for_key(nonce).ok()?;
    let plain = key(secret)
        .open_in_place(nonce, Aad::empty(), &mut data)
        .ok()?;
    Some(plain.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_and_open_round_trip() {
        let k = [7u8; 16];
        let s = seal(&k, b"\x06");
        assert_eq!(s.len(), 12 + 1 + 16);
        assert_eq!(open(&k, &s).as_deref(), Some(&b"\x06"[..]));
        assert_eq!(open(&[8u8; 16], &s), None);
        assert_eq!(open(&k, &s[..10]), None);
    }
}
