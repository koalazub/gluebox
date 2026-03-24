# Gluebox Refactor: Runtime-Configurable Service Daemon

## Problem

Gluebox hardcodes all connector logic. Adding or removing a service means editing source, recompiling, redeploying. No visibility into runtime state. No power management — the daemon burns CPU/network even when idle. Cloning the architecture for a new use case (studybot) requires forking and gutting, leading to fragmentation.

## Solution

Refactor gluebox into a runtime-configurable service daemon with:

- A `Connector` trait that all services implement for lifecycle management
- A `ConnectorRegistry` that starts/stops/reloads connectors based on config
- A LIF neuron-inspired power manager that transitions between active and resting states
- A ratatui TUI (`gluebox tui`) connected via unix socket with Cap'n Proto for zero-lag control
- HTTP admin API for scripting and nushell integration
- Hot-reload of `gluebox.toml` with full connector diffing

---

## Architecture

```
                      ┌─────────────────────────────────────┐
                      │           gluebox daemon             │
                      │                                      │
   webhooks ────────► │  axum HTTP server                    │
   admin REST ──────► │    /webhooks/*  /admin/*  /health    │
                      │                                      │
   gluebox tui ─────► │  unix socket ($XDG_RUNTIME_DIR/      │
     (capnp)          │    gluebox.sock)                     │
                      │                                      │
                      │  ┌──────────────────────────────┐    │
                      │  │  ConnectorRegistry            │    │
                      │  │  HashMap<name, Arc<dyn C>>    │    │
                      │  │                                │    │
                      │  │  linear   ● running            │    │
                      │  │  matrix   ◐ suspended          │    │
                      │  │  anytype  ○ stopped             │    │
                      │  │  github   ● running            │    │
                      │  └──────────────────────────────┘    │
                      │                                      │
                      │  ┌──────────────────────────────┐    │
                      │  │  LIF Power Manager            │    │
                      │  │  potential: 3.2 / 5.0         │    │
                      │  │  state: active                │    │
                      │  │  hysteresis: 2s min hold      │    │
                      │  └──────────────────────────────┘    │
                      │                                      │
                      │  ┌──────────────────────────────┐    │
                      │  │  AppState                     │    │
                      │  │  Arc<ConnectorRegistry>       │    │
                      │  │  Arc<Db>                      │    │
                      │  │  Arc<PowerManager>            │    │
                      │  │  Arc<RwLock<Config>>          │    │
                      │  └──────────────────────────────┘    │
                      └─────────────────────────────────────┘
```

---

## AppState

The central shared state passed to all handlers and triggers:

```rust
pub struct AppState {
    pub registry: Arc<ConnectorRegistry>,
    pub db: Arc<Db>,
    pub power: Arc<PowerManager>,
    pub config: Arc<RwLock<Config>>,
}
```

Webhook handlers read config for signature verification via `state.config.read()`. Triggers access connectors via `state.registry.get::<T>(name)`. Database access via `state.db`.

On hot-reload, the `config` RwLock is write-locked briefly to swap in the new config, then the registry diffs and applies connector changes.

---

## Connector Trait

Each connector manages its own interior mutability. All trait methods take `&self` so the registry never needs exclusive access to call lifecycle methods. Connectors use internal `Mutex<Inner>` or atomics for state.

```rust
pub trait Connector: Send + Sync + Any {
    fn name(&self) -> &'static str;
    fn status(&self) -> ConnectorStatus;
    fn as_any(&self) -> &dyn Any;

    async fn start(&self) -> anyhow::Result<()>;
    async fn stop(&self) -> anyhow::Result<()>;
    async fn suspend(&self) -> anyhow::Result<()>;
    async fn resume(&self) -> anyhow::Result<()>;
    async fn health_check(&self) -> anyhow::Result<()>;
    async fn reconfigure(&self, raw_toml: &toml::Value) -> anyhow::Result<bool>;
}

pub enum ConnectorStatus {
    Running,
    Stopped,
    Suspended,
    Error(String),
}
```

`as_any()` enables downcasting from `Arc<dyn Connector>` to concrete types like `Arc<LinearConnector>`.

`reconfigure()` receives the raw TOML sub-table for this connector's config section. The connector deserializes it internally into its typed config struct and applies the change. Returns `Ok(true)` if it handled the change without restart, `Ok(false)` if a full stop+start is needed. Default implementation returns `Ok(false)`.

Default implementations of `suspend`/`resume` delegate to `stop`/`start`. Matrix overrides to pause its sync loop without tearing down crypto state.

Triggers access domain-specific methods by downcasting:

```rust
let linear = state.registry.get::<LinearConnector>("linear")?;
linear.create_issue(&title, &body).await?;
```

Each connector struct holds its own config internally, wrapped in a `Mutex` for reconfiguration. Status is tracked via `AtomicU8` for lock-free reads from the TUI.

---

## ConnectorRegistry

```rust
pub struct ConnectorRegistry {
    connectors: RwLock<HashMap<String, Arc<dyn Connector>>>,
}
```

The RwLock protects the map itself (add/remove entries), not individual connector operations. Since all `Connector` methods take `&self`, concurrent reads of the map give `Arc<dyn Connector>` clones that can be used without holding the lock.

Operations:
- `register(name, connector)` — insert into map under write lock, then call `start()`
- `deregister(name)` — call `stop()`, then remove under write lock
- `get<T: Any>(name) -> Option<Arc<T>>` — read lock, clone Arc, downcast
- `toggle(name)` — read lock, check status, call `stop()` or `start()`
- `suspend_all()` / `resume_all()` — read lock, iterate, call suspend/resume
- `reload(old_config, new_config)` — diff and apply (see Hot-Reload)

---

## Hot-Reload

Three triggers:
1. TUI — press `r`
2. HTTP — `POST /admin/reload`
3. Signal — `SIGHUP`

Flow:
1. Parse new `gluebox.toml`. If invalid, log error, notify TUI, keep old config.
2. Diff each connector section against current using `PartialEq` on deserialized config structs (all config structs derive `PartialEq`).
3. For each connector:
   - Section appeared → construct connector, register, start
   - Section disappeared → stop, deregister
   - Section changed → try `reconfigure()` first. If it returns `false`, stop + reconstruct + start
   - Section unchanged → skip
4. Write-lock `state.config` briefly to swap in new config
5. Update power manager parameters if `[power]` section changed
6. Broadcast state update to connected TUI clients

Non-destructive: invalid config never replaces working config.

---

## Config Structure

All connector sections are `Option<T>`. Present means enabled, absent means disabled.

This is a breaking change from current gluebox where `linear`, `matrix`, and `documenso` are required. The migration wraps every access to these configs with Option checks. Webhook handlers that verify signatures check `state.config.read().linear.as_ref()` and return 503 if the connector isn't configured.

All config structs derive `Clone`, `Deserialize`, and `PartialEq`.

```toml
listen_addr = "127.0.0.1:8990"
# socket_path = "/run/user/1000/gluebox.sock"

# Required for admin API auth. If absent, admin endpoints reject all requests.
notify_secret = "bearer-token"

[turso]
url = "libsql://your-db.turso.io"
auth_token = "token"
# replica_path = "/var/lib/gluebox/gluebox.db"
# sync_interval_secs = 60
# encryption_key = "key"

[power]
threshold = 5.0
decay_rate = 0.5
tick_interval_secs = 30
spike_weight = 2.0
min_active_secs = 10
# Constraint: spike_weight must be > 0, decay_rate must be > 0
# threshold / spike_weight ≈ events-per-tick needed to stay active

# All connector sections optional. Present = enabled.
[linear]
api_key = "lin_api_XXX"
webhook_secret = "secret"

[matrix]
homeserver_url = "https://matrix.org"
access_token = "token"
room_id = "!room:matrix.org"
# bot_username = "bot"
# bot_password = "password"

[anytype]
api_url = "http://127.0.0.1:31012"
api_key = "key"
space_id = "space-id"

[documenso]
api_url = "https://app.documenso.com/api/v1"
api_key = "key"
webhook_secret = "secret"

[github]
token = "ghp_XXX"
repo = "owner/repo"
webhook_secret = "secret"

[opencode]
api_key = "openrouter-key"
```

---

## LIF Power Manager

Membrane potential model tracking daemon activity:

```
potential += spike_weight   (on each incoming event)
potential -= decay_rate     (each tick)
potential = max(0, potential)

if potential >= threshold AND state == Resting → transition to Active
if potential < threshold AND state == Active AND held for min_active_secs → transition to Resting
```

**Hysteresis**: `min_active_secs` prevents rapid oscillation around the threshold. Once active, the daemon stays active for at least this duration even if potential dips briefly.

**Parameter validation** (checked at config load and reload):
- `spike_weight > 0`
- `decay_rate > 0`
- `threshold > 0`
- `threshold / spike_weight` gives the approximate burst size needed to activate

**State transitions**:
- **Active → Resting**: potential below threshold for `min_active_secs`. Call `suspend_all()` on registry. TUI render rate drops to 2fps.
- **Resting → Active**: spike pushes potential above threshold. Call `resume_all()` on registry. TUI render rate jumps to 10fps.

**Matrix sync implication**: when resting, Matrix's background sync loop is paused. Matrix messages will not be received until an external event (webhook, TUI command, API call) fires a spike and wakes the daemon. This is intentional — low power means no polling.

Incoming webhooks always process regardless of power state. The webhook handler fires a spike, so sustained webhook traffic keeps the daemon active.

Implementation: `tokio::sync::watch` channel broadcasts power state changes. Registry and TUI subscribe.

---

## Unix Socket Protocol (Cap'n Proto)

Socket path: `$XDG_RUNTIME_DIR/gluebox.sock` (falls back to `/tmp/gluebox-<uid>.sock`). Configurable via `socket_path` in config. On startup, if socket file exists and no daemon is listening, unlink the stale file.

```capnp
struct Command {
  id @0 :UInt32;
  union {
    status @1 :Void;
    toggle @2 :Text;
    reload @3 :Void;
    spike @4 :Void;
    subscribe @5 :Void;
  }
}

struct CommandResponse {
  id @0 :UInt32;
  union {
    ok @1 :Void;
    error @2 :Text;
  }
}

struct StateSnapshot {
  uptimeSecs @0 :UInt64;
  potential @1 :Float64;
  threshold @2 :Float64;
  powerState @3 :PowerState;
  eventsPerMin @4 :Float32;
  connectors @5 :List(ConnectorState);
  framerate @6 :UInt8;
}

enum PowerState {
  active @0;
  resting @1;
}

struct ConnectorState {
  name @0 :Text;
  status @1 :Status;
  sparkline @2 :List(UInt8);
  eventCount @3 :UInt64;
  errorMessage @4 :Text;
}

enum Status {
  running @0;
  stopped @1;
  suspended @2;
  error @3;
}

struct ActivityEvent {
  timestampMs @0 :UInt64;
  source @1 :Text;
  eventType @2 :Text;
  detail @3 :Text;
}

struct DaemonMessage {
  union {
    state @0 :StateSnapshot;
    activity @1 :ActivityEvent;
    power @2 :PowerState;
    response @3 :CommandResponse;
  }
}
```

Daemon pushes `StateSnapshot` at 10fps active, 2fps resting. `ActivityEvent` streams inline as they happen. `CommandResponse` sent after each command for ack/error feedback.

---

## HTTP Admin API

On the existing axum server, gated behind `notify_secret` bearer token:

```
GET  /admin/status                    full state snapshot (JSON)
POST /admin/connectors/{name}/toggle  toggle connector on/off
POST /admin/reload                    hot-reload config
POST /admin/spike                     manual wake
GET  /admin/connectors                list connectors with status
GET  /admin/power                     LIF state
```

---

## TUI Layout (ratatui)

Single-screen dashboard. 10fps active, 2fps resting. Frame rate itself follows LIF — TUI interaction fires a spike.

```
┌─ GLUEBOX ──────────────────────────────────────────────────────────┐
│  ■ ACTIVE    ▁▂▃▅▇█▇▅▃▂▁▂▃▅▇  3.2/5.0    ↑ 12 evt/min    03:42 │
├────────────────────────────────┬───────────────────────────────────┤
│  Connectors                   │  Events                          │
│                                │                                  │
│  ● linear    ▂▃▅▃▂▁▂▃  42 evt │  linear   issue.created LIN-284 │
│  ◐ matrix    ▁▁▁▁▁▁▁▁   3 evt │  github   push main 3 commits   │
│  ● github    ▁▂▃▁▁▁▂▁  18 evt │  matrix   message !bot spec     │
│  ○ anytype                     │  linear   issue.updated LIN-281 │
│  ● documenso ▁▁▁▁▁▁▁▁   1 evt │  documenso doc.completed #89    │
│  ● opencode  ▁▁▂▁▁▁▁▁   7 evt │  linear   issue.created LIN-285 │
│                                │  github   issue.opened #42      │
│                                │  linear   comment.added LIN-284 │
│                                │                                  │
├────────────────────────────────┴───────────────────────────────────┤
│  [t]oggle  [r]eload  [q]uit  [↑↓] select                        │
└───────────────────────────────────────────────────────────────────┘
```

**Membrane potential bar**: oscilloscope-style waveform. Spikes cause sharp peaks with brief overshoot before settling. Decay follows a smooth exponential curve. Color gradient: deep blue (resting) → cyan → green → white/yellow (threshold) → warm amber (active). Threshold marked with a dashed indicator.

**Connector status icons**: `●` running (breathing pulse animation at ~1Hz), `◐` suspended (static half-circle), `○` stopped (hollow), `✖` error (flashing red at 2Hz).

**Sparklines**: 8-character wide per connector, showing request volume over last 8 time buckets.

**Event feed**: right panel with colored source tags (Linear purple, Matrix green, GitHub white, Documenso blue). New events bright, dim after a few seconds via ANSI dim. Auto-scrolling.

---

## Graceful Shutdown

On SIGTERM/SIGINT:
1. Stop accepting new webhook connections
2. Drain in-flight HTTP requests (axum graceful shutdown)
3. Call `stop()` on all connectors via registry
4. Close unix socket, remove socket file
5. Sync Turso replica if embedded replica is configured
6. Exit

SIGHUP triggers config reload (not shutdown).

---

## CLI Structure

```
gluebox              run the daemon (default)
gluebox tui          connect to running daemon, show dashboard
gluebox status       one-shot status dump (hits /admin/status)
gluebox reload       trigger hot-reload (hits /admin/reload)
gluebox toggle <n>   toggle a connector (hits /admin/connectors/{n}/toggle)
```

Parsed with `clap` subcommands.

---

## New Dependencies

| Crate | Purpose |
|-------|---------|
| `ratatui` | TUI framework |
| `crossterm` | Terminal backend for ratatui |
| `capnp` | Cap'n Proto runtime |
| `capnpc` | Cap'n Proto schema compiler (build dep) |
| `clap` | CLI subcommand parsing |

Native async fn in traits (edition 2024, Rust 1.75+). No `async-trait` crate needed.

Remove: `tokio-cron-scheduler` (unused).

---

## File Structure

```
src/
├── main.rs                    entry point, CLI dispatch
├── daemon.rs                  daemon startup, signal handling, shutdown
├── config.rs                  config structs (all derive PartialEq)
├── db.rs                      Turso database layer (unchanged)
├── power.rs                   LIF power manager
├── registry.rs                ConnectorRegistry
├── connector.rs               Connector trait + ConnectorStatus
├── socket.rs                  unix socket server (capnp framing)
├── connectors/
│   ├── mod.rs                 connector construction from config
│   ├── linear.rs              LinearConnector
│   ├── matrix.rs              MatrixConnector
│   ├── anytype.rs             AnytypeConnector
│   ├── documenso.rs           DocumensoConnector
│   ├── github.rs              GithubConnector
│   └── opencode.rs            OpenCodeConnector
├── triggers/
│   ├── mod.rs
│   ├── linear_to_anytype.rs
│   ├── linear_to_github.rs
│   ├── github_to_linear.rs
│   ├── documenso_handlers.rs
│   ├── to_matrix.rs
│   ├── anytype_to_linear.rs
│   └── reconcile.rs
├── webhook/
│   ├── mod.rs                 webhook routes + admin API routes
│   └── verify.rs              signature verification
├── tui/
│   ├── mod.rs                 TUI entry point + render loop
│   ├── layout.rs              panel layout
│   ├── waveform.rs            membrane potential visualization
│   ├── sparkline.rs           connector activity sparklines
│   └── event_feed.rs          event log panel
├── openclaw/
│   └── mod.rs                 Matrix AI bot
└── proto/
    └── gluebox.capnp          Cap'n Proto schema
```

---

## Migration Path

1. Add `Connector` trait and `ConnectorStatus` (`connector.rs`)
2. Wrap each existing connector to implement the trait (interior mutability via internal Mutex, AtomicU8 for status)
3. Build `ConnectorRegistry` with Arc-based storage and diff-based reload (`registry.rs`)
4. Make all config connector sections `Option<T>`, derive `PartialEq`, update all access sites
5. Build new `AppState` with `Arc<ConnectorRegistry>`, `Arc<Db>`, `Arc<PowerManager>`, `Arc<RwLock<Config>>`
6. Build `PowerManager` with watch channel and hysteresis (`power.rs`)
7. Add Cap'n Proto schema and unix socket server (`proto/`, `socket.rs`)
8. Add clap CLI with subcommands (`main.rs`)
9. Build TUI with ratatui — dashboard layout, waveform, sparklines, event feed (`tui/`)
10. Add HTTP admin routes to existing axum router
11. Wire SIGHUP/SIGTERM/SIGINT handlers (`daemon.rs`)
12. Update webhook handlers to pull connector config from `state.config.read()` for signature verification
13. Update trigger functions to pull connectors from registry via `state.registry.get::<T>(name)`
