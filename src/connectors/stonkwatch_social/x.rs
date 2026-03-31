use anyhow::{Context, Result};
use chrono::Utc;

use crate::config::XConfig;

pub async fn post_tweet(config: &XConfig, text: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let timestamp = Utc::now().timestamp().to_string();
    let nonce: String = (0..32)
        .map(|_| {
            let idx = rand::random::<u8>() % 36;
            if idx < 10 { (b'0' + idx) as char } else { (b'a' + idx - 10) as char }
        })
        .collect();

    let params = [
        ("oauth_consumer_key", config.api_key.as_str()),
        ("oauth_nonce", &nonce),
        ("oauth_signature_method", "HMAC-SHA1"),
        ("oauth_timestamp", &timestamp),
        ("oauth_token", &config.access_token),
        ("oauth_version", "1.0"),
    ];

    let param_string: String = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    let base_string = format!(
        "POST&{}&{}",
        urlencoding::encode("https://api.twitter.com/2/tweets"),
        urlencoding::encode(&param_string)
    );

    let signing_key = format!(
        "{}&{}",
        urlencoding::encode(&config.api_secret),
        urlencoding::encode(&config.access_secret)
    );

    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha1::Sha1;

    let mut mac = Hmac::<Sha1>::new_from_slice(signing_key.as_bytes())
        .map_err(|e| anyhow::anyhow!("HMAC init failed: {}", e))?;
    mac.update(base_string.as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

    let auth_header = format!(
        "OAuth oauth_consumer_key=\"{}\", oauth_nonce=\"{}\", oauth_signature=\"{}\", oauth_signature_method=\"HMAC-SHA1\", oauth_timestamp=\"{}\", oauth_token=\"{}\", oauth_version=\"1.0\"",
        urlencoding::encode(&config.api_key),
        urlencoding::encode(&nonce),
        urlencoding::encode(&signature),
        urlencoding::encode(&timestamp),
        urlencoding::encode(&config.access_token),
    );

    let body = serde_json::json!({ "text": text });

    let response = client
        .post("https://api.twitter.com/2/tweets")
        .header("Authorization", &auth_header)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("Failed to send tweet")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("X API error {}: {}", status, body);
    }

    let result: serde_json::Value = response.json().await?;
    Ok(result["data"]["id"].as_str().unwrap_or("unknown").to_string())
}
