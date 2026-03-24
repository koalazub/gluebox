# Gluebox Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor gluebox from a hardcoded webhook service into a runtime-configurable daemon with connector lifecycle management, LIF power manager, TUI dashboard, and hot-reload.

**Architecture:** Connectors implement a trait for lifecycle (`start`/`stop`/`suspend`/`resume`). A `ConnectorRegistry` holds `Arc<dyn Connector>` behind a `RwLock<HashMap>`. A LIF power manager transitions between active/resting states. TUI connects via unix socket using Cap'n Proto. HTTP admin API for scripting.

**Tech Stack:** Rust (edition 2024), axum, ratatui, crossterm, capnp, clap, tokio, tokio-util, libsql/Turso

**Spec:** `docs/superpowers/specs/2026-03-24-gluebox-refactor-design.md`

**Code style:** No code comments. Self-documenting names only.

---

## Phase 1: Foundation (Connector Trait + Registry + Config)

The core abstractions that everything else builds on. At the end of this phase, the daemon boots using the registry but behavior is identical to today.

### Task 1: Connector Trait and Status

**Files:**
- Create: `src/connector.rs`

- [ ] **Step 1: Write the connector trait**

```rust
// src/connector.rs
use std::any::Any;
use std::future::Future;
use std::pin::Pin;

pub enum ConnectorStatus {
    Running,
    Stopped,
    Suspended,
    Error(String),
}

impl ConnectorStatus {
    pub fn as_u8(&self) -> u8 {
        match self {
            Self::Running => 0,
            Self::Stopped => 1,
            Self::Suspended => 2,
            Self::Error(_) => 3,
        }
    }
}

pub trait Connector: Send + Sync {
    fn name(&self) -> &'static str;
    fn status(&self) -> ConnectorStatus;
    fn as_any(&self) -> &dyn Any;

    fn start(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;
    fn stop(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    fn suspend(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        self.stop()
    }
    fn resume(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        self.start()
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    fn reconfigure(
        &self,
        _raw_toml: &toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>> {
        Box::pin(async { Ok(false) })
    }
}
```

- [ ] **Step 2: Add module to main.rs**

Add `mod connector;` to `src/main.rs` after existing mod declarations.

- [ ] **Step 3: Run cargo check**

Run: `cargo check`
Expected: compiles with no errors

- [ ] **Step 4: Commit**

```
jj describe -m "feat: add Connector trait and ConnectorStatus"
```

---

### Task 2: ConnectorRegistry

**Files:**
- Create: `src/registry.rs`

- [ ] **Step 1: Write the registry**

```rust
// src/registry.rs
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::connector::{Connector, ConnectorStatus};

pub struct ConnectorRegistry {
    connectors: RwLock<HashMap<String, Arc<dyn Connector>>>,
}

impl ConnectorRegistry {
    pub fn new() -> Self {
        Self {
            connectors: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register(&self, name: String, connector: Arc<dyn Connector>) -> anyhow::Result<()> {
        connector.start().await?;
        self.connectors.write().await.insert(name, connector);
        Ok(())
    }

    pub async fn deregister(&self, name: &str) -> anyhow::Result<Option<Arc<dyn Connector>>> {
        let conn = self.connectors.write().await.remove(name);
        if let Some(ref c) = conn {
            c.stop().await?;
        }
        Ok(conn)
    }

    pub async fn get_dyn_typed<T: Connector + 'static>(&self, name: &str) -> Option<Arc<dyn Connector>> {
        let lock = self.connectors.read().await;
        let conn = lock.get(name)?;
        conn.as_any().downcast_ref::<T>().is_some().then(|| conn.clone())
    }

    pub async fn get_dyn(&self, name: &str) -> Option<Arc<dyn Connector>> {
        self.connectors.read().await.get(name).cloned()
    }

    pub async fn toggle(&self, name: &str) -> anyhow::Result<ConnectorStatus> {
        let conn = self
            .connectors
            .read()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("connector not found: {name}"))?;

        match conn.status() {
            ConnectorStatus::Running => {
                conn.stop().await?;
                Ok(conn.status())
            }
            ConnectorStatus::Stopped | ConnectorStatus::Suspended => {
                conn.start().await?;
                Ok(conn.status())
            }
            ConnectorStatus::Error(_) => {
                conn.start().await?;
                Ok(conn.status())
            }
        }
    }

    pub async fn suspend_all(&self) {
        let lock = self.connectors.read().await;
        for (name, conn) in lock.iter() {
            if let ConnectorStatus::Running = conn.status() {
                if let Err(e) = conn.suspend().await {
                    tracing::error!("failed to suspend {name}: {e}");
                }
            }
        }
    }

    pub async fn resume_all(&self) {
        let lock = self.connectors.read().await;
        for (name, conn) in lock.iter() {
            if let ConnectorStatus::Suspended = conn.status() {
                if let Err(e) = conn.resume().await {
                    tracing::error!("failed to resume {name}: {e}");
                }
            }
        }
    }

    pub async fn stop_all(&self) {
        let lock = self.connectors.read().await;
        for (name, conn) in lock.iter() {
            match conn.status() {
                ConnectorStatus::Stopped => {}
                _ => {
                    if let Err(e) = conn.stop().await {
                        tracing::error!("failed to stop {name}: {e}");
                    }
                }
            }
        }
    }

    pub async fn list(&self) -> Vec<(String, ConnectorStatus)> {
        let lock = self.connectors.read().await;
        lock.iter()
            .map(|(name, conn)| (name.clone(), conn.status()))
            .collect()
    }

    pub async fn names(&self) -> Vec<String> {
        self.connectors.read().await.keys().cloned().collect()
    }
}
```

- [ ] **Step 2: Add module to main.rs**

Add `mod registry;` to `src/main.rs`.

- [ ] **Step 3: Run cargo check**

Run: `cargo check`
Expected: compiles (registry is defined but not yet wired in)

- [ ] **Step 4: Commit**

```
jj describe -m "feat: add ConnectorRegistry with Arc<dyn Connector> storage"
```

---

### Task 3: Make Config Fully Optional + PartialEq

**Files:**
- Modify: `src/config.rs`

This is the breaking change. All connector config sections become `Option<T>`. All config structs derive `PartialEq`.

- [ ] **Step 1: Rewrite config.rs**

Change `src/config.rs`:
- Add `PartialEq` derive to ALL config structs
- Change `linear: LinearConfig` → `linear: Option<LinearConfig>`
- Change `matrix: MatrixConfig` → `matrix: Option<MatrixConfig>`
- Change `documenso: DocumensoConfig` → `documenso: Option<DocumensoConfig>`
- Add `socket_path: Option<String>` field to Config
- Add `power: Option<PowerConfig>` field to Config
- Add PowerConfig struct: `threshold: f64, decay_rate: f64, tick_interval_secs: u64, spike_weight: f64, min_active_secs: u64` with defaults
- Keep `turso` as required (not Option)

- [ ] **Step 2: Run cargo check, collect all errors**

Run: `cargo check 2>&1 | head -100`
Expected: errors everywhere that accesses `cfg.linear`, `cfg.matrix`, `cfg.documenso` directly

- [ ] **Step 3: Fix webhook/mod.rs**

Every direct access to `state.cfg.linear`, `state.cfg.matrix`, `state.cfg.documenso` needs `.as_ref()` with appropriate error handling. Return `StatusCode::SERVICE_UNAVAILABLE` when config is absent.

Breakage sites:
- `handle_linear`: `state.cfg.linear.webhook_secret` → wrap with `.as_ref().ok_or(SERVICE_UNAVAILABLE)?`
- `handle_documenso`: `state.cfg.documenso.webhook_secret` (line ~174) → same pattern
- `handle_notify`: `state.cfg.matrix.room_id` (line ~375) → wrap with `.as_ref()`
- `handle_github`: already uses Option pattern, verify

- [ ] **Step 4: Fix triggers/**

All breakage sites for direct config access:
- `linear_to_anytype.rs` line ~63: `state.cfg.linear.api_key` → `.as_ref().ok_or_else(|| anyhow!("linear not configured"))?`
- `to_matrix.rs` line ~12: `state.cfg.matrix.feedback_room_id` → wrap with `state.cfg.matrix.as_ref().and_then(|m| m.feedback_room_id.as_deref())`
- `to_matrix.rs` line ~26: `state.cfg.matrix.issues_room_id` → same pattern
- `documenso_handlers.rs` line ~107: `state.cfg.linear.api_key` (NOT documenso config) → wrap with `.as_ref()`
- `documenso_handlers.rs` lines ~26,82: `state.cfg.anytype` → already Option, verify
- `github_to_linear.rs` lines ~19,21: `state.cfg.linear.api_key`, `state.cfg.linear.team_id` → wrap
- `linear_to_github.rs`: verify handles Option
- `anytype_to_linear.rs` line ~14: `state.cfg.linear.api_key` → wrap (dead code but must still compile)
- `reconcile.rs` line ~20: `state.cfg.linear.api_key` → wrap (dead code but must compile)

- [ ] **Step 5: Fix main.rs**

Update matrix bot init: `cfg.matrix` is now `Option<MatrixConfig>`. Wrap the entire init block in `if let Some(ref matrix_cfg) = cfg.matrix { ... }`.
Update anytype init: already Optional, verify.

- [ ] **Step 6: Fix openclaw/mod.rs**

Breakage sites:
- Line ~168: `state.cfg.linear.api_key` → `state.cfg.linear.as_ref().ok_or_else(|| anyhow!("linear not configured"))?.api_key`
- Line ~240: `state.cfg.linear.api_key` → same pattern
- Line ~242: `state.cfg.linear.team_id` → same pattern

- [ ] **Step 7: Run cargo check**

Run: `cargo check`
Expected: compiles clean

- [ ] **Step 8: Run tests**

Run: `cargo test`
Expected: all existing tests pass

- [ ] **Step 9: Commit**

```
jj describe -m "refactor: make all connector configs optional, derive PartialEq"
```

---

### Task 4: New AppState + Wire Registry Into main.rs

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Rewrite AppState and main()**

Replace the current `AppState` with:

```rust
pub struct AppState {
    pub registry: Arc<ConnectorRegistry>,
    pub db: Arc<db::Db>,
    pub config: Arc<RwLock<config::Config>>,
}
```

Add `use tokio::sync::RwLock;` and `use crate::registry::ConnectorRegistry;`.

Update `main()`:
- Create `ConnectorRegistry::new()`
- Wrap db in `Arc::new()`
- Wrap config in `Arc::new(RwLock::new(cfg))`
- Build AppState with these
- For now, keep the old connector initialization inline (matrix bot, anytype ensure_types) — we'll move these into the registry in Phase 2

- [ ] **Step 2: Update all references to old AppState fields**

Every file that uses `state.cfg` now needs `state.config.read().await` or `state.config.read().await.clone()` for the specific field. This touches:
- `webhook/mod.rs`: all handlers
- `triggers/*`: all trigger functions
- `openclaw/mod.rs`: all config reads

For webhook handlers (sync context in axum), use `state.config.read().await` at the top of the handler and bind it.

- [ ] **Step 3: Run cargo check**

Run: `cargo check`
Expected: compiles

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: all pass

- [ ] **Step 5: Commit**

```
jj describe -m "refactor: new AppState with Arc<ConnectorRegistry>, Arc<RwLock<Config>>"
```

---

## Phase 2: Wrap Existing Connectors

Each existing connector gets a wrapper implementing the `Connector` trait. Interior mutability via `tokio::sync::Mutex<Option<Inner>>` — `None` when stopped, `Some(client)` when running.

### Task 5: Wrap LinearConnector

**Files:**
- Modify: `src/connectors/linear.rs`

- [ ] **Step 1: Add Connector trait impl**

Keep the existing `LinearClient` struct and all its methods unchanged. Add a `LinearConnector` wrapper:

```rust
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::sync::Mutex;

pub struct LinearConnector {
    config: Mutex<crate::config::LinearConfig>,
    client: Mutex<Option<LinearClient>>,
    status: AtomicU8,
    error_msg: Mutex<Option<String>>,
}
```

Implement `Connector` for `LinearConnector`:
- `start()`: create `LinearClient` from config, store in `self.client`, set status to Running
- `stop()`: drop client (set to None), set status to Stopped
- `status()`: read `AtomicU8`, if error read `error_msg`
- `health_check()`: verify client exists
- `reconfigure()`: deserialize `toml::Value` into `LinearConfig`, compare with current, update if changed, return `Ok(true)` (no restart needed for API key changes since client is recreated per-request anyway)

Add a `pub fn client(&self)` method that returns a clone of the inner `LinearClient` (or error if stopped). This is what triggers call.

- [ ] **Step 2: Run cargo check**

- [ ] **Step 3: Commit**

```
jj describe -m "feat: wrap LinearClient with Connector trait"
```

---

### Task 6: Wrap Remaining Connectors

**Files:**
- Modify: `src/connectors/matrix.rs`
- Modify: `src/connectors/anytype.rs`
- Modify: `src/connectors/documenso.rs`
- Modify: `src/connectors/github.rs`
- Modify: `src/connectors/opencode.rs`

- [ ] **Step 1: Wrap each connector**

Same wrapper pattern as Task 5 (inner `Mutex<Option<Client>>`, `AtomicU8` status, `Mutex<Option<String>>` for error message). Key differences per connector:

**MatrixConnector**: `start()` calls `MatrixBot::login()` + `initial_sync()` + spawns `sync_forever` as a tokio task. The sync task is wrapped in `tokio::select!` with a `tokio_util::sync::CancellationToken`. `suspend()` cancels the token (pausing sync) but keeps the `MatrixBot` alive with its crypto state. `resume()` spawns a new sync task with a fresh token. `stop()` cancels the token AND drops the `MatrixBot`. Store the `CancellationToken` alongside the bot.

**AnytypeConnector**: `start()` creates `AnytypeClient` and calls `ensure_types()`. Simple reqwest wrapper otherwise.

**DocumensoConnector**: this connector has NO client struct — `documenso.rs` only contains webhook payload types. The `DocumensoConnector` wrapper is a status-only connector: `start()` just sets status to Running, `stop()` sets to Stopped. Its config (webhook_secret) is read from `state.config` during webhook verification. No inner client needed.

**GithubConnector, OpenCodeConnector**: simple reqwest-based wrappers like LinearConnector.

- [ ] **Step 1b: Add tokio-util dependency**

Add to Cargo.toml: `tokio-util = { version = "0.7", features = ["rt"] }` (for `CancellationToken` used by MatrixConnector).

- [ ] **Step 2: Update connectors/mod.rs**

Re-export the `*Connector` wrapper types alongside the existing client types.

- [ ] **Step 3: Run cargo check**

- [ ] **Step 4: Run tests**

- [ ] **Step 5: Commit**

```
jj describe -m "feat: wrap all connectors with Connector trait"
```

---

### Task 7: Register Connectors in main.rs

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace inline connector init with registry registration**

In `main()`, after creating the registry:
- For each `Option<*Config>` that is `Some`, construct the corresponding `*Connector`, wrap in `Arc`, register in the registry, call `start()`.
- Remove the old inline matrix bot init, anytype ensure_types, etc.
- The matrix bot `Arc` previously stored in `AppState` now lives inside `MatrixConnector`.

- [ ] **Step 2: Update openclaw startup**

The openclaw task previously took `Arc<MatrixBot>`. Now it should get the bot from the registry: `state.registry.get::<MatrixConnector>("matrix")`.

- [ ] **Step 3: Run cargo check**

- [ ] **Step 4: Run tests**

- [ ] **Step 5: Commit**

```
jj describe -m "feat: register all connectors via registry on startup"
```

---

### Task 8: Update Triggers to Use Registry

**Files:**
- Modify: `src/triggers/linear_to_anytype.rs`
- Modify: `src/triggers/linear_to_github.rs`
- Modify: `src/triggers/github_to_linear.rs`
- Modify: `src/triggers/documenso_handlers.rs`
- Modify: `src/triggers/to_matrix.rs`
- Modify: `src/webhook/mod.rs`

- [ ] **Step 1: Update each trigger**

Replace inline connector construction like:
```rust
let linear = LinearClient::new(&state.cfg.linear.api_key);
```
With registry access:
```rust
let linear = state.registry.get::<LinearConnector>("linear")
    .await
    .ok_or_else(|| anyhow::anyhow!("linear connector not available"))?;
let client = linear.client()?;
```

Do this for every trigger file. Each connector's wrapper exposes a `.client()` method that returns the inner API client.

- [ ] **Step 2: Update webhook handlers**

Webhook signature verification still needs config. Read from `state.config.read().await`. The handler itself dispatches to triggers which use the registry.

- [ ] **Step 3: Run cargo check**

- [ ] **Step 4: Run tests**

- [ ] **Step 5: Commit**

```
jj describe -m "refactor: triggers use registry instead of inline connector construction"
```

---

## Phase 3: LIF Power Manager

### Task 9: Power Manager

**Files:**
- Create: `src/power.rs`

- [ ] **Step 1: Write power manager with tests**

```rust
// src/power.rs
use std::sync::Arc;
use tokio::sync::watch;

pub enum PowerState {
    Active,
    Resting,
}

// Uses crate::config::PowerConfig from config.rs (same struct, no duplication).
// config::PowerConfig derives Deserialize + PartialEq for hot-reload diffing.

pub struct PowerManager {
    potential: std::sync::Mutex<f64>,
    config: std::sync::Mutex<crate::config::PowerConfig>,
    state_tx: watch::Sender<PowerState>,
    state_rx: watch::Receiver<PowerState>,
    last_active_at: std::sync::Mutex<std::time::Instant>,
}
```

Methods:
- `new(config: PowerConfig) -> anyhow::Result<Self>` — validates config params before constructing
- `spike(&self)` — add spike_weight to potential, check threshold crossing
- `tick(&self)` — subtract decay_rate, check threshold crossing with hysteresis
- `state(&self) -> PowerState` — current state
- `subscribe(&self) -> watch::Receiver<PowerState>` — for registry and TUI
- `potential(&self) -> f64` — for TUI display
- `reconfigure(&self, config: PowerConfig) -> anyhow::Result<()>` — validates then applies

The `tick()` method is called from a background tokio task at `tick_interval_secs` intervals.

Threshold crossing logic:
- Resting → Active: when potential >= threshold after a spike
- Active → Resting: when potential < threshold AND time since last spike > min_active_secs

**Parameter validation** (enforced in `new()` and `reconfigure()`):
- `spike_weight > 0.0`
- `decay_rate > 0.0`
- `threshold > 0.0`
- `tick_interval_secs > 0`

Returns error if any constraint violated.

- [ ] **Step 2: Write tests**

Test spike accumulation, decay, threshold crossing, hysteresis.

- [ ] **Step 3: Run tests**

Run: `cargo test power`
Expected: all pass

- [ ] **Step 4: Commit**

```
jj describe -m "feat: LIF power manager with hysteresis"
```

---

### Task 10: Wire Power Manager Into Daemon

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add PowerManager to AppState**

Add `pub power: Arc<PowerManager>` to `AppState`. Initialize from `cfg.power` (with defaults if absent). Spawn the tick background task. Subscribe registry to power state changes — on `Active` call `resume_all()`, on `Resting` call `suspend_all()`.

- [ ] **Step 2: Add spike() calls to webhook handlers**

In `webhook/mod.rs`, add `state.power.spike()` at the start of every webhook handler and API endpoint.

- [ ] **Step 3: Run cargo check**

- [ ] **Step 4: Run tests**

- [ ] **Step 5: Commit**

```
jj describe -m "feat: wire LIF power manager into daemon lifecycle"
```

---

## Phase 4: CLI + Admin API + Hot-Reload

### Task 11: Clap CLI

**Files:**
- Modify: `src/main.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add clap dependency**

Add to Cargo.toml: `clap = { version = "4", features = ["derive"] }`

- [ ] **Step 2: Add CLI parsing to main.rs**

```rust
#[derive(clap::Parser)]
#[command(name = "gluebox")]
enum Cli {
    #[command(hide = true)]
    Daemon,
    Tui,
    Status,
    Reload,
    Toggle { connector: String },
}
```

Default (no subcommand) runs the daemon. Other subcommands are HTTP client calls to `/admin/*` that print results and exit. TUI subcommand is a placeholder for now (Phase 5).

- [ ] **Step 3: Run cargo check**

- [ ] **Step 4: Commit**

```
jj describe -m "feat: clap CLI with daemon/tui/status/reload/toggle subcommands"
```

---

### Task 12: HTTP Admin API

**Files:**
- Modify: `src/webhook/mod.rs`

- [ ] **Step 1: Add admin routes**

Add to the router:
```rust
.route("/admin/status", get(admin_status))
.route("/admin/connectors", get(admin_connectors))
.route("/admin/connectors/{name}/toggle", post(admin_toggle))
.route("/admin/reload", post(admin_reload))
.route("/admin/spike", post(admin_spike))
.route("/admin/power", get(admin_power))
```

Each handler checks `notify_secret` bearer token. Returns JSON.

`admin_status`: returns full state — power state, potential, connectors list, uptime.
`admin_connectors`: returns connector names + statuses.
`admin_toggle`: calls `state.registry.toggle(name)`.
`admin_reload`: triggers config reload (see Task 13).
`admin_spike`: calls `state.power.spike()`.
`admin_power`: returns LIF state.

- [ ] **Step 2: Run cargo check**

- [ ] **Step 3: Commit**

```
jj describe -m "feat: HTTP admin API for status/toggle/reload/spike"
```

---

### Task 13: Hot-Reload + Signal Handling + Daemon Module

**Files:**
- Create: `src/daemon.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write daemon.rs**

Contains:
- `reload(state: &AppState) -> anyhow::Result<()>`: re-reads config file, diffs, applies changes to registry
- Signal handling setup: SIGHUP → reload, SIGTERM/SIGINT → graceful shutdown

Reload diff logic:
1. Parse new config
2. For each connector type, compare `Option<XConfig>` old vs new using `PartialEq`
3. If `None → Some`: construct + register + start
4. If `Some → None`: stop + deregister
5. If `Some(old) != Some(new)`: try `reconfigure()`, if false then stop + deregister + construct + register + start
6. If equal: skip
7. Swap config under write lock
8. Update power config if changed

- [ ] **Step 2: Wire into main.rs**

Move daemon startup logic from `main()` into `daemon::run()`. `main()` just does CLI dispatch. The daemon function sets up signals before starting the server.

Graceful shutdown sequence on SIGTERM/SIGINT:
1. Stop accepting new connections (axum graceful shutdown)
2. Drain in-flight HTTP requests
3. Call `registry.stop_all()`
4. Close unix socket, remove socket file
5. If Turso embedded replica is configured, call `db.sync()` to flush
6. Exit

- [ ] **Step 3: Run cargo check**

- [ ] **Step 4: Run tests**

- [ ] **Step 5: Commit**

```
jj describe -m "feat: hot-reload with config diffing, SIGHUP/SIGTERM handling"
```

---

## Phase 5: Cap'n Proto Socket + TUI

### Task 14: Cap'n Proto Schema

**Files:**
- Create: `src/proto/gluebox.capnp`
- Create: `build.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add capnp dependencies**

Cargo.toml:
```toml
capnp = "0.20"

[build-dependencies]
capnpc = "0.20"
```

- [ ] **Step 2: Write the schema**

Create `src/proto/gluebox.capnp` with the full schema from the spec (Command, CommandResponse, StateSnapshot, ConnectorState, ActivityEvent, DaemonMessage, PowerState, Status enums).

- [ ] **Step 3: Write build.rs**

```rust
fn main() {
    capnpc::CompilerCommand::new()
        .file("src/proto/gluebox.capnp")
        .run()
        .expect("capnp compile");
}
```

- [ ] **Step 4: Run cargo check**

Verify schema compiles and generates Rust code.

- [ ] **Step 5: Commit**

```
jj describe -m "feat: Cap'n Proto schema for TUI ↔ daemon protocol"
```

---

### Task 15: Unix Socket Server

**Files:**
- Create: `src/socket.rs`

- [ ] **Step 1: Write the socket server**

Listens on the configured socket path (default `$XDG_RUNTIME_DIR/gluebox.sock`). Accepts connections. For each connection:
- Read `Command` messages
- Dispatch to registry/power/daemon as appropriate
- Send `CommandResponse` for each command
- After `subscribe` command, push `StateSnapshot` at frame rate and `ActivityEvent` inline

Uses `capnp::serialize` for framing over the unix stream.

On startup: check if socket file exists, try connecting — if no daemon listening, unlink stale file.

- [ ] **Step 2: Add event broadcast channel**

Create a `tokio::sync::broadcast` channel for `ActivityEvent`. Webhook handlers and triggers push events to this channel. Socket server forwards to subscribed TUI clients.

- [ ] **Step 3: Wire into daemon.rs**

Spawn socket server task during daemon startup. Clean up socket file on shutdown.

- [ ] **Step 4: Run cargo check**

- [ ] **Step 5: Commit**

```
jj describe -m "feat: unix socket server with capnp framing"
```

---

### Task 16: TUI — Layout and Render Loop

**Files:**
- Create: `src/tui/mod.rs`
- Create: `src/tui/layout.rs`
- Modify: `Cargo.toml`
- Modify: `src/main.rs`

- [ ] **Step 1: Add ratatui + crossterm dependencies**

```toml
ratatui = "0.29"
crossterm = "0.28"
```

- [ ] **Step 2: Write TUI entry point**

`src/tui/mod.rs`: connects to unix socket, subscribes to state stream, enters crossterm raw mode + alternate screen, runs render loop.

Frame rate: 10fps (100ms tick) when active, 2fps (500ms tick) when resting. Reads power state from `StateSnapshot` to determine rate.

Input handling: `crossterm::event::poll` with the tick duration. Key events: `t` toggle, `r` reload, `q` quit, up/down arrow for connector selection. Every key press also sends a `spike` command to the daemon so TUI interaction keeps the daemon awake.

- [ ] **Step 3: Write layout**

`src/tui/layout.rs`: three-row layout using `ratatui::layout::Layout`:
- Top row: power state, waveform placeholder, stats, time
- Middle row: two columns — connectors (left), events (right)
- Bottom row: keybind help bar

For now, render placeholder text in each panel. Visuals come in Tasks 17-19.

- [ ] **Step 4: Wire TUI subcommand**

In `main.rs`, the `Cli::Tui` variant calls `tui::run()`.

- [ ] **Step 5: Run cargo check**

- [ ] **Step 6: Commit**

```
jj describe -m "feat: TUI skeleton with layout and render loop"
```

---

### Task 17: TUI — Waveform Visualization

**Files:**
- Create: `src/tui/waveform.rs`

- [ ] **Step 1: Write the membrane potential waveform widget**

A custom ratatui widget that:
- Maintains a rolling buffer of potential values (last N frames)
- Renders as a sparkline-style bar using Unicode block characters (▁▂▃▄▅▆▇█)
- Color interpolation: deep blue (0%) → cyan (25%) → green (50%) → white/yellow (75% = threshold) → amber (100%)
- Threshold marker rendered as a dim dashed line at the threshold position
- On spike: value jumps sharply. On decay: smooth exponential visual

The widget receives `potential` and `threshold` from `StateSnapshot` each frame and appends to its ring buffer.

- [ ] **Step 2: Integrate into layout**

Replace the top-row placeholder with the waveform widget.

- [ ] **Step 3: Run cargo check**

- [ ] **Step 4: Commit**

```
jj describe -m "feat: TUI membrane potential waveform visualization"
```

---

### Task 18: TUI — Connector Panel with Sparklines

**Files:**
- Create: `src/tui/sparkline.rs`
- Modify: `src/tui/layout.rs`

- [ ] **Step 1: Write connector panel rendering**

For each connector in `StateSnapshot.connectors`:
- Status icon with animation state:
  - `●` Running: color cycles between bright and slightly dim at ~1Hz (use frame counter mod)
  - `◐` Suspended: static yellow
  - `○` Stopped: dim gray
  - `✖` Error: alternates red/dark red at 2Hz
- Inline sparkline from the `sparkline` field (8 values → 8 Unicode block chars)
- Event count right-aligned
- Selected row highlighted with a subtle background color

- [ ] **Step 2: Integrate into layout left column**

- [ ] **Step 3: Run cargo check**

- [ ] **Step 4: Commit**

```
jj describe -m "feat: TUI connector panel with sparklines and animated status icons"
```

---

### Task 19: TUI — Event Feed

**Files:**
- Create: `src/tui/event_feed.rs`
- Modify: `src/tui/layout.rs`

- [ ] **Step 1: Write event feed panel**

Maintains a ring buffer of recent `ActivityEvent` entries. Each entry:
- Source tag colored by connector (Linear=purple, Matrix=green, GitHub=white, Documenso=blue, Anytype=cyan, OpenCode=yellow)
- Event type + detail truncated to fit
- Age-based dimming: events older than 5 seconds get `Style::new().dim()`
- Auto-scrolls to newest

- [ ] **Step 2: Integrate into layout right column**

- [ ] **Step 3: Run cargo check**

- [ ] **Step 4: Commit**

```
jj describe -m "feat: TUI event feed with colored source tags and age dimming"
```

---

## Phase 6: Cleanup + README

### Task 20: Remove Dead Code + Update Dependencies

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/triggers/mod.rs`

- [ ] **Step 1: Remove unused dependencies**

Remove `tokio-cron-scheduler` from Cargo.toml (unused).
Check if `urlencoding` is still referenced — remove if not.

- [ ] **Step 2: Clean up dead code markers**

Remove `#[allow(dead_code)]` from `anytype_to_linear.rs` and `reconcile.rs` if they're now properly wired through the registry. If still unused, remove the files entirely.

- [ ] **Step 3: Run cargo check and test**

- [ ] **Step 4: Commit**

```
jj describe -m "chore: remove dead code and unused dependencies"
```

---

### Task 21: Update README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Rewrite README**

Cover:
- What gluebox is (runtime-configurable service daemon)
- Architecture overview (connector trait, registry, LIF power, TUI)
- Quick start: config file, `gluebox` to start daemon, `gluebox tui` for dashboard
- Config reference: all sections with examples
- CLI reference: all subcommands
- Adding a new connector: implement trait, add config section, register in `connectors/mod.rs`

- [ ] **Step 2: Commit**

```
jj describe -m "docs: update README for refactored architecture"
```

---

### Task 22: Update Example Config

**Files:**
- Modify: `gluebox.example.toml`

- [ ] **Step 1: Add new sections**

Add `socket_path`, `[power]` section with all parameters and their defaults explained. Add example future connector section (commented out `[affine]`).

- [ ] **Step 2: Commit**

```
jj describe -m "docs: update example config with power and socket sections"
```

---

## Verification Checklist

After all tasks:

- [ ] `cargo check` — clean compile, no warnings
- [ ] `cargo test` — all tests pass
- [ ] `gluebox` starts the daemon with a valid config
- [ ] `gluebox tui` connects and shows the dashboard
- [ ] `gluebox status` prints current state
- [ ] `gluebox toggle linear` toggles a connector
- [ ] `gluebox reload` reloads config
- [ ] Edit config, reload → new connector appears in TUI
- [ ] Remove config section, reload → connector disappears
- [ ] Daemon goes idle → transitions to Resting state
- [ ] Webhook hits → transitions back to Active
- [ ] SIGTERM → graceful shutdown
- [ ] SIGHUP → config reload
