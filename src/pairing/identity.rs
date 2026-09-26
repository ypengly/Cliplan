use rand::RngCore;
use sha2::{Digest, Sha256};

/// Generate a random 256-bit device token. This is the *only* time the
/// plaintext token ever exists outside the requesting device's browser --
/// the server stores only its hash (see `hash_token`), matching the "no
/// permanent credentials in URLs/QR codes, use keys/tokens" requirement.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 hash of a token, stored in place of the token itself so a
/// database leak does not hand out valid credentials.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Human-friendly fingerprint shown to the admin when approving a pairing
/// request, e.g. `7F:2A:91:4C:B3:1D`. Derived from a fresh random value,
/// not from anything the connecting device sent us, so it can't be spoofed
/// to look like a previously-trusted device.
pub fn generate_fingerprint() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Generate a short numeric pairing code, e.g. `483920`, for manual /
/// cross-check entry alongside the QR scan.
pub fn generate_code() -> String {
    let n = rand::thread_rng().next_u32() % 1_000_000;
    format!("{n:06}")
}

/// Constant-time-ish string comparison for codes/tokens to reduce timing
/// side channels on comparison of secrets.
pub fn secure_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= x ^ y;
    }
    diff == 0
}
