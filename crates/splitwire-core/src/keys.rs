//! WireGuard (Curve25519) key helpers. Keys are 32 bytes, base64-encoded.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use x25519_dalek::{PublicKey, StaticSecret};

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("key is not valid base64")]
    Base64,
    #[error("key must decode to 32 bytes")]
    Length,
}

fn decode(key: &str) -> Result<[u8; 32], KeyError> {
    let bytes = STANDARD.decode(key.trim()).map_err(|_| KeyError::Base64)?;
    bytes.try_into().map_err(|_| KeyError::Length)
}

pub fn is_valid_key(key: &str) -> bool {
    decode(key).is_ok()
}

/// A new private key, clamped the same way as `wg genkey`.
pub fn generate_private_key() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS random number generator failed");
    bytes[0] &= 248;
    bytes[31] &= 127;
    bytes[31] |= 64;
    STANDARD.encode(bytes)
}

pub fn public_key(private_key: &str) -> Result<String, KeyError> {
    let secret = StaticSecret::from(decode(private_key)?);
    Ok(STANDARD.encode(PublicKey::from(&secret).as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_known_public_key() {
        // RFC 7748 section 6.1 test vector (Alice).
        let private = STANDARD.encode(
            hex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a"),
        );
        let public = STANDARD.encode(
            hex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"),
        );
        assert_eq!(public_key(&private).unwrap(), public);
    }

    #[test]
    fn generated_keys_are_valid() {
        let private = generate_private_key();
        assert!(is_valid_key(&private));
        assert!(is_valid_key(&public_key(&private).unwrap()));
        assert!(!is_valid_key("abc"));
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
}
