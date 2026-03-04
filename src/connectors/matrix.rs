use anyhow::Result;
use matrix_sdk::{
    Client,
    config::SyncSettings,
    encryption::EncryptionSettings,
    ruma::{
        OwnedRoomId,
        api::client::uiaa,
        events::room::message::RoomMessageEventContent,
    },
};
use std::path::PathBuf;

pub struct MatrixBot {
    client: Client,
    room_id: OwnedRoomId,
}

impl MatrixBot {
    pub async fn login(
        homeserver_url: &str,
        username: &str,
        password: &str,
        room_id: &str,
        data_dir: PathBuf,
    ) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)?;

        let client = Self::connect(homeserver_url, username, password, &data_dir).await?;

        client.encryption().wait_for_e2ee_initialization_tasks().await;

        let needs_bootstrap = match client.encryption().cross_signing_status().await {
            Some(status) if status.is_complete() => {
                tracing::info!("matrix-sdk: cross-signing already complete");
                false
            }
            _ => true,
        };

        if needs_bootstrap {
            tracing::info!("matrix-sdk: bootstrapping cross-signing (step 1: no auth)...");
            match client.encryption().bootstrap_cross_signing(None).await {
                Ok(()) => {
                    tracing::info!("matrix-sdk: cross-signing bootstrapped (no UIAA required)");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "matrix-sdk: first bootstrap attempt returned error, trying with password");
                    let mut pwd = uiaa::Password::new(
                        uiaa::UserIdentifier::UserIdOrLocalpart(username.to_owned()),
                        password.to_owned(),
                    );
                    if let Some(response) = e.as_uiaa_response() {
                        pwd.session = response.session.clone();
                    }
                    match client
                        .encryption()
                        .bootstrap_cross_signing(Some(uiaa::AuthData::Password(pwd)))
                        .await
                    {
                        Ok(()) => tracing::info!("matrix-sdk: cross-signing bootstrapped with password"),
                        Err(e) => tracing::error!(error = %e, "matrix-sdk: cross-signing bootstrap failed with password"),
                    }
                }
            }
        }

        let room_id: OwnedRoomId = room_id.parse()?;

        Ok(Self { client, room_id })
    }

    async fn connect(
        homeserver_url: &str,
        username: &str,
        password: &str,
        data_dir: &std::path::Path,
    ) -> Result<Client> {
        let client = Self::build_client(homeserver_url, data_dir).await?;

        if let Some(session) = client.session() {
            tracing::info!(
                user = %session.meta().user_id,
                device = %session.meta().device_id,
                "matrix-sdk: restoring persisted session"
            );
            return Ok(client);
        }

        tracing::info!(user = %username, "matrix-sdk: no persisted session, logging in fresh");
        match client
            .matrix_auth()
            .login_username(username, password)
            .initial_device_display_name("gluebox-bot")
            .send()
            .await
        {
            Ok(_) => {
                tracing::info!(
                    user = %username,
                    device = ?client.device_id(),
                    "matrix-sdk: logged in, session will persist in sqlite store"
                );
                Ok(client)
            }
            Err(e) if e.to_string().contains("crypto store") || e.to_string().contains("account in the store") => {
                tracing::warn!(error = %e, "matrix-sdk: stale crypto store, wiping and retrying");
                drop(client);
                let _ = std::fs::remove_dir_all(data_dir);
                std::fs::create_dir_all(data_dir)?;
                let fresh = Self::build_client(homeserver_url, data_dir).await?;
                fresh
                    .matrix_auth()
                    .login_username(username, password)
                    .initial_device_display_name("gluebox-bot")
                    .send()
                    .await?;
                tracing::info!(user = %username, "matrix-sdk: logged in with fresh crypto store");
                Ok(fresh)
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn build_client(homeserver_url: &str, data_dir: &std::path::Path) -> Result<Client> {
        Ok(Client::builder()
            .homeserver_url(homeserver_url)
            .sqlite_store(data_dir, None)
            .with_encryption_settings(EncryptionSettings {
                auto_enable_cross_signing: true,
                ..Default::default()
            })
            .build()
            .await?)
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn room_id(&self) -> &OwnedRoomId {
        &self.room_id
    }

    pub async fn send_message(&self, body: &str) -> Result<()> {
        let room = self.client.get_room(&self.room_id)
            .ok_or_else(|| anyhow::anyhow!("room not found: {}", self.room_id))?;

        let content = RoomMessageEventContent::text_plain(body);
        room.send(content).await?;
        Ok(())
    }

    pub async fn send_markdown(&self, markdown: &str) -> Result<()> {
        let room = self.client.get_room(&self.room_id)
            .ok_or_else(|| anyhow::anyhow!("room not found: {}", self.room_id))?;

        let html = markdown_to_html(markdown);
        let content = RoomMessageEventContent::text_html(markdown, html);
        room.send(content).await?;
        Ok(())
    }

    pub async fn send_to_room(&self, target_room_id: &str, body: &str) -> Result<()> {
        let room_id: OwnedRoomId = target_room_id.parse()?;
        let room = self.client.get_room(&room_id)
            .ok_or_else(|| anyhow::anyhow!("room not found: {}", room_id))?;

        let content = RoomMessageEventContent::text_plain(body);
        room.send(content).await?;
        Ok(())
    }

    pub async fn send_markdown_to_room(&self, target_room_id: &str, markdown: &str) -> Result<()> {
        let room_id: OwnedRoomId = target_room_id.parse()?;
        let room = self.client.get_room(&room_id)
            .ok_or_else(|| anyhow::anyhow!("room not found: {}", room_id))?;

        let html = markdown_to_html(markdown);
        let content = RoomMessageEventContent::text_html(markdown, html);
        room.send(content).await?;
        Ok(())
    }

    pub async fn initial_sync(&self) -> Result<()> {
        tracing::info!("matrix-sdk: running initial sync...");
        self.client.sync_once(SyncSettings::default()).await?;
        tracing::info!("matrix-sdk: initial sync complete");
        Ok(())
    }

    pub async fn sync_forever(&self, settings: SyncSettings) {
        self.client.sync(settings).await.ok();
    }
}

fn markdown_to_html(md: &str) -> String {
    md.replace("\n\n", "<br><br>")
      .replace("**", "<strong>")
      .replace("__", "<em>")
      .replace("`", "<code>")
}
