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

    constant_time_eq_pub(&computed, &header_bytes)
}

pub fn github_signature(header_sig: &str, raw_body: &[u8], secret: &str) -> bool {
    let Some(hex_part) = header_sig.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(header_bytes) = hex::decode(hex_part) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(raw_body);
    constant_time_eq_pub(&mac.finalize().into_bytes(), &header_bytes)
}

pub fn documenso_secret(header_secret: &str, expected: &str) -> bool {
    constant_time_eq_pub(header_secret.as_bytes(), expected.as_bytes())
}

pub fn constant_time_eq_pub(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    fn sign(body: &[u8], secret: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn linear_sig_valid() {
        let sig = sign(b"payload", "secret");
        assert!(linear_signature(&sig, b"payload", "secret"));
    }

    #[test]
    fn linear_sig_wrong_secret() {
        let sig = sign(b"payload", "correct");
        assert!(!linear_signature(&sig, b"payload", "wrong"));
    }

    #[test]
    fn linear_sig_tampered_body() {
        let sig = sign(b"original", "secret");
        assert!(!linear_signature(&sig, b"tampered", "secret"));
    }

    #[test]
    fn linear_sig_invalid_hex() {
        assert!(!linear_signature("not-hex!!", b"body", "secret"));
    }

    #[test]
    fn documenso_match() {
        assert!(documenso_secret("my-secret", "my-secret"));
    }

    #[test]
    fn documenso_wrong() {
        assert!(!documenso_secret("wrong", "my-secret"));
    }

    #[test]
    fn documenso_different_length() {
        assert!(!documenso_secret("short", "much-longer-secret"));
    }

    #[test]
    fn ct_eq_equal() {
        assert!(constant_time_eq_pub(b"hello", b"hello"));
    }

    #[test]
    fn ct_eq_different() {
        assert!(!constant_time_eq_pub(b"hello", b"world"));
    }

    #[test]
    fn ct_eq_different_lengths() {
        assert!(!constant_time_eq_pub(b"hi", b"hello"));
    }

    #[test]
    fn ct_eq_empty() {
        assert!(constant_time_eq_pub(b"", b""));
    }

    #[test]
    fn github_sig_valid() {
        let sig = format!("sha256={}", sign(b"payload", "secret"));
        assert!(github_signature(&sig, b"payload", "secret"));
    }

    #[test]
    fn github_sig_wrong_secret() {
        let sig = format!("sha256={}", sign(b"payload", "correct"));
        assert!(!github_signature(&sig, b"payload", "wrong"));
    }

    #[test]
    fn github_sig_missing_prefix() {
        let sig = sign(b"payload", "secret");
        assert!(!github_signature(&sig, b"payload", "secret"));
    }

    #[test]
    fn github_sig_tampered_body() {
        let sig = format!("sha256={}", sign(b"original", "secret"));
        assert!(!github_signature(&sig, b"tampered", "secret"));
    }
}
