use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn linear_signature(header_sig: &str, raw_body: &[u8], secret: &str) -> bool {
    let Ok(header_bytes) = hex::decode(header_sig) else {
        return false;
    };

    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };

    mac.update(raw_body);
    let computed = mac.finalize().into_bytes();

    constant_time_eq(&computed, &header_bytes)
}

pub fn documenso_secret(header_secret: &str, expected: &str) -> bool {
    constant_time_eq(header_secret.as_bytes(), expected.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
