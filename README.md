# Gluebox

A runtime-configurable service daemon that synchronises data across external tools and reacts to events. Connectors are managed through a common lifecycle interface, toggled on or off at runtime, and hot-reloaded from a single TOML config file. A leaky integrate-and-fire (LIF) neuron model governs power state, suspending idle connectors and waking them on demand. A TUI dashboard provides real-time visibility into connector health, activity, and power state.

Gluebox currently integrates: Linear, Anytype, Matrix, Documenso, GitHub, OpenCode (AI), AFFine, and a filesystem watcher for Hyprnote/char lecture sessions.

---

## Table of Contents

- [Architecture](#architecture)
- [Connectors](#connectors)
- [Triggers](#triggers)
- [Studybot: Lecture Capture and Study Planning](#studybot-lecture-capture-and-study-planning)
- [Power Management](#power-management)
- [TUI Dashboard](#tui-dashboard)
- [Admin API](#admin-api)
- [Webhook Endpoints](#webhook-endpoints)
- [CLI](#cli)
- [Configuration](#configuration)
- [Hot-Reload](#hot-reload)
- [Production Deployment](#production-deployment)
- [Adding a New Connector](#adding-a-new-connector)
- [Building](#building)

---

## Architecture

The system is built around four core abstractions:

**Connector trait.** Every external service implements `Connector`, which provides lifecycle methods (`start`, `stop`, `suspend`, `resume`, `health_check`, `reconfigure`). All methods take `&self` and return boxed futures, making the trait object-safe. Connectors manage their own interior mutability.

**ConnectorRegistry.** Holds `Arc<dyn Connector>` instances in an async `RwLock<HashMap>`. Provides `register` (which calls `start`), `deregister` (which calls `stop`), `toggle`, `suspend_all`, `resume_all`, `stop_all`, and `list`. Triggers and webhook handlers pull connectors from the registry by name.

**PowerManager.** Models a LIF neuron. Incoming events call `spike()`, raising membrane potential. A background ticker calls `tick()`, decaying potential. When potential crosses a configurable threshold, the daemon transitions to Active and resumes all suspended connectors. When it decays back below threshold (after a hysteresis hold period), it transitions to Resting and suspends them.

**Unix socket with Cap'n Proto.** The daemon listens on a unix socket. The TUI connects here and receives a zero-copy stream of state snapshots and activity events. Commands (toggle, reload, spike) flow from the TUI back to the daemon over the same socket.

---

## Connectors

Each connector wraps an API client and implements the Connector trait. All connector config sections in `gluebox.toml` are optional. Present means enabled; absent means disabled.

| Connector | Service | Purpose |
|-----------|---------|---------|
| `linear` | Linear | Issue tracking, spec management, feedback tickets |
| `anytype` | Anytype | Object storage for specs and contracts |
| `matrix` | Matrix | E2EE messaging, notifications, AI chatbot |
| `documenso` | Documenso | Document signing webhook payloads |
| `github` | GitHub | Issue sync with Linear |
| `opencode` | OpenRouter | AI-powered spec drafting, feedback clustering |
| `affine` | AFFine Cloud | Lecture notes and study plan documents |
| `watcher` | Filesystem | Monitors Hyprnote sessions for new lecture recordings |

Matrix is the only connector with distinct suspend/resume behaviour. Suspending pauses its long-polling sync loop while preserving E2EE crypto state. All other connectors tear down and reconstruct on suspend/resume.

---

## Triggers

Triggers are the business logic that runs when events arrive. They pull connectors from the registry, perform cross-service operations, and persist state to the database.

| Trigger | Source | Action |
|---------|--------|--------|
| `linear_issue_created` | Linear webhook | Creates Anytype spec object, stores mapping |
| `linear_issue_updated` | Linear webhook | Updates Anytype spec, notifies Matrix on state changes |
| `github_issue_opened` | GitHub webhook | Creates Linear issue, stores bidirectional mapping |
| `linear_issue_github_sync` | Linear webhook | Creates GitHub issue from Linear, stores mapping |
| `documenso_completed` | Documenso webhook | Creates/updates Anytype contract, notifies Matrix |
| `documenso_rejected` | Documenso webhook | Updates contract status, notifies Matrix |
| `session_import` | Watcher / API | Imports char lecture session to AFFine |
| `study_plan` | API | Generates priority-ranked study plan in AFFine |

---

## Studybot: Lecture Capture and Study Planning

Gluebox includes connectors for an automated lecture capture and study planning pipeline.

**How it works.** The `watcher` connector monitors Hyprnote (char) session directories for new `_summary.md` files. When a summary appears, the watcher debounces the event (default 30 seconds), then checks whether the recording session overlaps with a university calendar event by matching the session timestamp against events in Hyprnote's `events.json`. Non-university recordings are skipped.

For matched university sessions, the `session_import` trigger reads the session summary, constructs a formatted lecture page, and pushes it to AFFine Cloud via the `affine` connector. The import is recorded in the database to prevent duplicates.

The `study_plan` trigger gathers recent imported lectures, builds a priority-ranked markdown document with lecture summaries and a task checklist, and creates it as an AFFine page.

**API endpoints** for manual control:

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/sessions` | List all imported sessions |
| POST | `/api/import` | Import the latest unimported uni session |
| POST | `/api/import/all` | Batch import all unimported uni sessions |
| POST | `/api/import/{session_id}` | Import a specific session by ID |
| POST | `/api/study-plan` | Generate a study plan (accepts `period` and `course` in body) |

All study API endpoints require bearer token authentication via `notify_secret`.

**Configuration:**

```toml
[affine]
api_url = "https://app.affine.pro"
api_token = "your-token"
workspace_id = "your-workspace-id"

[watcher]
sessions_dir = "~/Library/Application Support/hyprnote/sessions"
hyprnote_dir = "~/Library/Application Support/hyprnote"
debounce_secs = 30
uni_calendar_names = ["Uni"]
```

---

## Power Management

The power manager models a leaky integrate-and-fire neuron:

1. Each incoming event (webhook, API call, TUI interaction) fires a spike, adding `spike_weight` to the membrane potential.
2. A background ticker subtracts `decay_rate` from potential every `tick_interval_secs` seconds, floored at zero.
3. When potential reaches `threshold`, the daemon transitions to **Active**. All suspended connectors are resumed.
4. When potential decays below `threshold` and the daemon has been active for at least `min_active_secs`, it transitions to **Resting**. Connectors are suspended. The HTTP server remains live so incoming webhooks still process and fire spikes.

This means a sustained burst of webhooks keeps the daemon fully active. When traffic stops, the daemon gradually powers down. A single webhook during resting state processes normally and wakes the daemon if repeated.

All parameters are configurable in `[power]` and hot-reloadable.

---

## TUI Dashboard

`gluebox tui` connects to the daemon's unix socket and renders a live terminal dashboard.

```
+-- GLUEBOX ----------------------------------------------------------+
|  ACTIVE    [membrane potential waveform]  3.2/5.0  12 evt/min  03:42|
+-------------------------------+-------------------------------------+
|  Connectors                   |  Events                             |
|  * linear    ........  42 evt |  linear   issue.created LIN-284     |
|  ~ matrix    ........   3 evt |  github   push main 3 commits       |
|  * github    ........  18 evt |  matrix   message !bot spec          |
|  o anytype                    |  linear   issue.updated LIN-281      |
|  * affine    ........   5 evt |  affine   doc.created Study Plan     |
+-------------------------------+-------------------------------------+
|  [t]oggle  [r]eload  [q]uit  [up/down] select                      |
+---------------------------------------------------------------------+
```

The waveform visualises membrane potential as an oscilloscope trace with colour gradient (blue at rest through to amber at full activity). Connector status icons animate: running connectors pulse, error states flash. Each connector shows an inline sparkline of recent activity. Events dim with age.

Frame rate: 10fps when active, 2fps when resting.

---

## Admin API

HTTP endpoints on the daemon's `listen_addr`, gated behind the `notify_secret` bearer token.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/admin/status` | Full state snapshot (power, connectors, uptime) |
| GET | `/admin/connectors` | List connectors with status |
| POST | `/admin/connectors/{name}/toggle` | Toggle a connector on/off |
| POST | `/admin/reload` | Hot-reload config from disk |
| POST | `/admin/spike` | Manually fire a power spike |
| GET | `/admin/power` | Current LIF state (potential, threshold) |
| GET | `/health` | Health check |

---

## Webhook Endpoints

External services send events to these endpoints. Each verifies the request signature before processing.

| Path | Service | Verification |
|------|---------|-------------|
| `/webhooks/linear` | Linear | HMAC-SHA256 via `linear-signature` header |
| `/webhooks/documenso` | Documenso | Constant-time secret comparison |
| `/webhooks/github` | GitHub | HMAC-SHA256 with `sha256=` prefix |

---

## CLI

| Command | Description |
|---------|-------------|
| `gluebox` | Run the daemon (default when no subcommand given) |
| `gluebox tui` | Connect to a running daemon and show the TUI dashboard |
| `gluebox status` | Print current state as JSON (hits `/admin/status`) |
| `gluebox reload` | Trigger config reload (hits `/admin/reload`) |
| `gluebox toggle <name>` | Toggle a connector (hits `/admin/connectors/{name}/toggle`) |

The `status`, `reload`, and `toggle` subcommands are HTTP clients that hit `http://127.0.0.1:8990`. If `listen_addr` is customised, use the admin API directly.

---

## Configuration

Configuration lives in a single TOML file. Default path: `gluebox.toml` in the working directory. Override with `GLUEBOX_CONFIG=/path/to/file.toml`.

See `gluebox.example.toml` for a complete annotated example.

| Section | Purpose |
|---------|---------|
| *(root)* | `listen_addr`, `notify_secret`, `socket_path` |
| `[turso]` | Database connection. Required. Supports `libsql://` (Turso cloud), `file:` (local SQLite), or remote with local replica via `replica_path`. |
| `[power]` | LIF parameters: `threshold`, `decay_rate`, `tick_interval_secs`, `spike_weight`, `min_active_secs` |
| `[linear]` | Linear API key, webhook secret, optional team ID |
| `[anytype]` | Anytype HTTP API URL, API key, space ID |
| `[matrix]` | Matrix homeserver, access token, room IDs, optional bot credentials for E2EE |
| `[documenso]` | Documenso API URL, API key, webhook secret |
| `[opencode]` | OpenRouter API key (enables the AI chatbot in Matrix) |
| `[github]` | GitHub token, repo, webhook secret |
| `[affine]` | AFFine Cloud API URL, token, workspace ID |
| `[watcher]` | Hyprnote session directory paths, debounce timing, university calendar names |

All connector sections are optional. Present means enabled. Absent means disabled. Add or remove sections and reload to change what runs.

---

## Hot-Reload

Three ways to trigger a config reload:

1. **TUI** -- press `r`
2. **HTTP** -- `POST /admin/reload`
3. **Signal** -- send `SIGHUP` to the daemon process

The reload reads `gluebox.toml` from disk, diffs each connector section against the running config using `PartialEq`, and applies changes:

- Section appeared: construct and start the connector.
- Section disappeared: stop and deregister.
- Section changed: attempt `reconfigure()`. If the connector cannot reconfigure in place, it is stopped and restarted with the new config.
- Section unchanged: no action.

If the new config file is invalid, the reload fails safely. The daemon continues with the previous config and reports the error.

---

## Production Deployment

Gluebox runs on a NixOS VPS with the configuration in `hosts/gluebox-prod/`. The flake provides:

- `nix build .#gluebox` -- the release binary
- `nixosConfigurations.gluebox-prod` -- full NixOS system config
- `deploy.nodes.gluebox-prod` -- deploy-rs activation profile

The production stack includes Tailscale for networking (funnel exposes the webhook endpoints), Valkey with bloom filter module, MongoDB with replica set for Anytype, the any-sync-bundle server, and anytype-cli for the Anytype HTTP API.

The daemon runs as a systemd service that restarts on failure with a 30-second delay. Config lives at `/etc/gluebox/gluebox.toml`.

CI runs `cargo nextest` on every push. The deploy workflow builds the NixOS closure and activates it via deploy-rs over SSH.

---

## Adding a New Connector

1. Create `src/connectors/<name>.rs` with a client struct and a connector wrapper that implements `Connector`. Follow `src/connectors/linear.rs` as the reference pattern: `Mutex<Option<Client>>` for interior mutability, `AtomicU8` for status, boxed futures on all trait methods.

2. Add a config struct to `src/config.rs` with `Debug, Clone, Serialize, Deserialize, PartialEq` derives. Add an `Option<YourConfig>` field to `Config`.

3. Add `pub mod <name>;` to `src/connectors/mod.rs`.

4. Add a registration block in `main.rs` (guarded by `if let Some(ref cfg) = cfg.<name>`).

5. Add a reload diff block in `daemon.rs` following the existing pattern.

6. Add the TOML section to your config file and reload.

---

## Building

```sh
cargo build --release
```

With Nix:

```sh
nix build .#gluebox
```

Development shell (drops into nushell with all dependencies):

```sh
nix develop
```

Run tests:

```sh
cargo test
# or with nextest
cargo nextest run
```
