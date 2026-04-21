use std::any::Any;
use std::path::PathBuf;
use std::pin::Pin;
use std::future::Future;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::process::Command;
use tokio::sync::Mutex;
use gluebox_core::{Connector, ConnectorStatus};

pub struct SamayaConnector {
    config: Mutex<crate::config::SamayaConfig>,
    status: AtomicU8,
    error_msg: Mutex<Option<String>>,
    child: Mutex<Option<tokio::process::Child>>,
}

impl SamayaConnector {
    pub fn new(config: crate::config::SamayaConfig) -> Self {
        Self {
            config: Mutex::new(config),
            status: AtomicU8::new(ConnectorStatus::Stopped.as_u8()),
            error_msg: Mutex::new(None),
            child: Mutex::new(None),
        }
    }

    pub async fn start_recording(&self, output_path: PathBuf) -> anyhow::Result<()> {
        let config = self.config.lock().await;
        let binary = config.binary.clone();
        drop(config);

        let mut guard = self.child.lock().await;
        if guard.is_some() {
            anyhow::bail!("samaya is already recording");
        }

        let child = Command::new(&binary)
            .arg("listen")
            .arg("-o")
            .arg(&output_path)
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn samaya: {e}"))?;

        tracing::info!(output = %output_path.display(), "samaya recording started");
        *guard = Some(child);
        Ok(())
    }

    pub async fn stop_recording(&self) -> anyhow::Result<()> {
        let mut guard = self.child.lock().await;
        if let Some(child) = guard.as_mut() {
            if let Some(pid) = child.id() {
                // SAFETY: sending SIGINT to a child process we own
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGINT);
                }
            }
            child.wait().await.ok();
            tracing::info!("samaya recording stopped");
        }
        *guard = None;
        Ok(())
    }

    pub async fn is_recording(&self) -> bool {
        self.child.lock().await.is_some()
    }
}

impl Connector for SamayaConnector {
    fn name(&self) -> &'static str {
        "samaya"
    }

    fn status(&self) -> ConnectorStatus {
        match self.status.load(Ordering::SeqCst) {
            0 => ConnectorStatus::Running,
            1 => ConnectorStatus::Stopped,
            2 => ConnectorStatus::Suspended,
            _ => {
                let msg = self.error_msg.blocking_lock()
                    .clone()
                    .unwrap_or_default();
                ConnectorStatus::Error(msg)
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn start(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let config = self.config.lock().await;
            let binary = config.binary.clone();
            let output_dir = config.output_dir.clone();
            drop(config);

            // Verify samaya binary exists
            let result = Command::new(&binary)
                .arg("--version")
                .output()
                .await;

            match result {
                Ok(output) if output.status.success() => {
                    tracing::info!(
                        version = %String::from_utf8_lossy(&output.stdout).trim(),
                        "samaya binary verified"
                    );
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    anyhow::bail!("samaya --version failed: {stderr}");
                }
                Err(e) => {
                    anyhow::bail!("samaya binary '{}' not accessible: {e}", binary);
                }
            }

            // Create output directory
            if let Err(e) = tokio::fs::create_dir_all(&output_dir).await {
                anyhow::bail!("failed to create samaya output dir {}: {e}", output_dir.display());
            }

            self.status.store(ConnectorStatus::Running.as_u8(), Ordering::SeqCst);
            tracing::info!(output_dir = %output_dir.display(), "samaya connector started");
            Ok(())
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            self.stop_recording().await?;
            self.status.store(ConnectorStatus::Stopped.as_u8(), Ordering::SeqCst);
            tracing::info!("samaya connector stopped");
            Ok(())
        })
    }

    fn suspend(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            self.stop_recording().await?;
            self.status.store(ConnectorStatus::Suspended.as_u8(), Ordering::SeqCst);
            tracing::info!("samaya connector suspended");
            Ok(())
        })
    }

    fn resume(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        self.start()
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let config = self.config.lock().await;
            let binary = config.binary.clone();
            drop(config);

            let result = Command::new(&binary)
                .arg("--version")
                .output()
                .await;

            match result {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    anyhow::bail!("samaya binary unhealthy: {stderr}");
                }
                Err(e) => {
                    anyhow::bail!("samaya binary not accessible: {e}");
                }
            }
        })
    }

    fn reconfigure(
        &self,
        raw_toml: &toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>> {
        let raw_toml = raw_toml.clone();
        Box::pin(async move {
            let new_cfg: crate::config::SamayaConfig = raw_toml.try_into()
                .map_err(|e| anyhow::anyhow!("failed to parse samaya config: {e}"))?;
            *self.config.lock().await = new_cfg;
            tracing::info!("samaya connector reconfigured");
            Ok(true)
        })
    }
}
