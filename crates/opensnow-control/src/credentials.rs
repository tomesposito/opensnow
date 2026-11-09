//! Salted credential hashing. Passwords use PBKDF2-HMAC-SHA256 built on the
//! locked `hmac` and `sha2` crates. Tokens are high entropy random values, so
//! a single SHA-256 hash is the right storage form for them. Plaintext
//! passwords and tokens are never stored.
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;
const PBKDF2_ITERATIONS: u32 = 100_000;
const SALT_BYTES: usize = 16;
const TOKEN_BYTES: usize = 32;

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
}

/// PBKDF2-HMAC-SHA256 with a 32 byte derived key (RFC 8018, one block).
fn pbkdf2(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(password).expect("HMAC accepts any key length");
    mac.update(salt);
    mac.update(&1u32.to_be_bytes());
    let mut u = mac.finalize().into_bytes();
    let mut out: [u8; 32] = u.into();
    for _ in 1..iterations.max(1) {
        let mut mac = HmacSha256::new_from_slice(password).expect("HMAC accepts any key length");
        mac.update(&u);
        u = mac.finalize().into_bytes();
        for (o, b) in out.iter_mut().zip(u.iter()) { *o ^= b; }
    }
    out
}

/// Returns `pbkdf2-sha256$<iterations>$<salt hex>$<hash hex>` for storage.
pub fn hash_password(password: &str) -> String {
    let salt = random_bytes::<SALT_BYTES>();
    let hash = pbkdf2(password.as_bytes(), &salt, PBKDF2_ITERATIONS);
    format!("pbkdf2-sha256${PBKDF2_ITERATIONS}${}${}", hex::encode(salt), hex::encode(hash))
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    if parts.len() != 4 || parts[0] != "pbkdf2-sha256" { return false; }
    let Ok(iterations) = parts[1].parse::<u32>() else { return false };
    if iterations == 0 || iterations > 1_000_000 { return false; }
    let (Ok(salt), Ok(expected)) = (hex::decode(parts[2]), hex::decode(parts[3])) else { return false };
    if salt.is_empty() || salt.len() > 64 || expected.len() != 32 { return false; }
    let actual = pbkdf2(password.as_bytes(), &salt, iterations);
    actual.ct_eq(&expected).into()
}

/// A stored value that never matches. Used so a missing login still pays the
/// verification cost and the failure surface stays uniform.
pub fn dummy_password_hash() -> String {
    format!("pbkdf2-sha256${PBKDF2_ITERATIONS}${}${}", hex::encode([0u8; SALT_BYTES]), hex::encode([0u8; 32]))
}

/// One-time plaintext token. Shown to the caller once and never stored.
pub fn generate_token() -> String { format!("osn_{}", hex::encode(random_bytes::<TOKEN_BYTES>())) }

/// Storage form of a token: hex encoded SHA-256 of the plaintext.
pub fn hash_token(plaintext: &str) -> String { hex::encode(Sha256::digest(plaintext.as_bytes())) }

pub fn token_hash_matches(plaintext: &str, stored_hash: &str) -> bool {
    match hex::decode(stored_hash) {
        Ok(expected) => Sha256::digest(plaintext.as_bytes()).as_slice().ct_eq(&expected).into(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip_and_wrong_password_fails() {
        let stored = hash_password("correct horse battery staple");
        assert!(!stored.contains("correct horse"));
        assert!(verify_password("correct horse battery staple", &stored));
        assert!(!verify_password("wrong", &stored));
        assert!(!verify_password("correct horse battery staple", "not-a-hash"));
        assert!(!verify_password("anything", &dummy_password_hash()));
    }

    #[test]
    fn password_salts_differ() {
        assert_ne!(hash_password("same-password"), hash_password("same-password"));
    }

    #[test]
    fn token_hash_is_not_plaintext() {
        let token = generate_token();
        let stored = hash_token(&token);
        assert_ne!(stored, token);
        assert!(!stored.contains(&token));
        assert!(token_hash_matches(&token, &stored));
        assert!(!token_hash_matches("osn_wrong", &stored));
    }
}
