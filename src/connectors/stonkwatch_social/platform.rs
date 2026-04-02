use anyhow::Result;
use std::future::Future;
use std::pin::Pin;

pub struct SocialPost {
    pub text: String,
    pub link: String,
    pub image_url: Option<String>,
    pub og_title: String,
    pub og_description: String,
}

pub struct PostResult {
    pub platform: &'static str,
    pub id: String,
}

pub trait SocialPlatform: Send + Sync {
    fn name(&self) -> &'static str;
    fn publish<'a>(&'a self, post: &'a SocialPost) -> Pin<Box<dyn Future<Output = Result<PostResult>> + Send + 'a>>;
}

pub async fn check_response(response: reqwest::Response, platform: &str) -> Result<reqwest::Response> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("{} API error {}: {}", platform, status, body);
    }
    Ok(response)
}
