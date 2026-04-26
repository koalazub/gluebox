use anyhow::{Context, Result};
use chrono::Utc;
use tracing::info;

use crate::config::StorjConfig;

pub async fn upload_image(config: &StorjConfig, local_path: &str, object_key: &str) -> Result<String> {
    let bytes = tokio::fs::read(local_path).await
        .with_context(|| format!("Failed to read image at {}", local_path))?;

    let now = Utc::now();
    let date_stamp = now.format("%Y%m%d").to_string();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let region = "us-east-1";
    let service = "s3";

    let host = config.endpoint.trim_start_matches("https://").trim_start_matches("http://");
    let url = format!("{}/{}/{}", config.endpoint, config.bucket, object_key);

    let content_sha256 = sha256_hex(&bytes);

    let canonical_headers = format!("host:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\n", host, content_sha256, amz_date);
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";

    let canonical_request = format!(
        "PUT\n/{}/{}\n\n{}\n{}\n{}",
        config.bucket, object_key, canonical_headers, signed_headers, content_sha256
    );

    let credential_scope = format!("{}/{}/{}/aws4_request", date_stamp, region, service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date, credential_scope, sha256_hex(canonical_request.as_bytes())
    );

    let signing_key = derive_signing_key(&config.secret_key, &date_stamp, region, service);
    let signature = hmac_sha256_hex(&signing_key, string_to_sign.as_bytes());

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        config.access_key, credential_scope, signed_headers, signature
    );

    let client = reqwest::Client::new();
    let response = client
        .put(&url)
        .header("Authorization", &authorization)
        .header("x-amz-content-sha256", &content_sha256)
        .header("x-amz-date", &amz_date)
        .header("Content-Type", "image/png")
        .header("x-amz-acl", "public-read")
        .body(bytes)
        .send()
        .await
        .context("Failed to upload to Storj")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Storj upload error {}: {}", status, body);
    }

    let public_url = format!("{}/{}/{}/{}", config.public_base_url, config.access_key, config.bucket, object_key);
    info!(key = object_key, url = %public_url, "Uploaded OG image to Storj");
    Ok(public_url)
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    hex::encode(Sha256::digest(data))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    hex::encode(hmac_sha256(key, data))
}

fn derive_signing_key(secret_key: &str, date_stamp: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{}", secret_key).as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_produces_valid_hash() {
        let hash = sha256_hex(b"hello");
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    #[test]
    fn signing_key_derivation_is_deterministic() {
        let key1 = derive_signing_key("secret", "20260402", "us-east-1", "s3");
        let key2 = derive_signing_key("secret", "20260402", "us-east-1", "s3");
        assert_eq!(key1, key2);
        assert_eq!(key1.len(), 32);
    }
}
