# Gluebox

Runtime-configurable service daemon that syncs data across tools. Connectors for Linear, Anytype, Matrix, Documenso, and GitHub are managed through a common lifecycle interface. A LIF neuron-inspired power manager governs active/resting state. A ratatui TUI provides a live dashboard. Config hot-reloads on SIGHUP.

## Architecture

### Connector trait

Every integration implements `Connector`:

```rust
pub trait Connector: Send + Sync {
    fn name(&self) -> &'static str;
    fn status(&self) -> ConnectorStatus;
    fn start(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;
    fn stop(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;
    fn health_check(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;
    fn reconfigure(&self, raw_toml: &toml::Value) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + '_>>;
    // suspend/resume default to stop/start
}
```

`ConnectorStatus` is one of `Running`, `Stopped`, `Suspended`, or `Error(String)`.

### ConnectorRegistry

`ConnectorRegistry` holds a `HashMap<String, Arc<dyn Connector>>` behind an async `RwLock`. It provides `register`, `deregister`, `toggle`, `suspend_all`, `resume_all`, `stop_all`, and `list`.

### LIF power manager

`PowerManager` models a leaky integrate-and-fire neuron. Each incoming activity event calls `spike()`, which adds `spike_weight` to the membrane potential. A background ticker calls `tick()` on each interval, decaying the potential by `decay_rate`. When potential crosses `threshold` the manager transitions to `Active` and notifies all subscribers via a `watch` channel. When potential decays back below `threshold` and `min_active_secs` have elapsed, it transitions to `Resting`. The registry suspends all connectors on `Resting` and resumes them on `Active`.

### Unix socket + Cap'n Proto

The daemon listens on a unix socket (default `gluebox.sock` in the working directory, overridden by `socket_path`). Messages are framed with Cap'n Proto. The TUI connects to this socket to stream live connector status and activity events.

### HTTP admin API

Axum serves an HTTP admin API on `listen_addr`:

- `GET  /admin/status` — JSON snapshot of all connector statuses and power state
- `POST /admin/reload` — trigger config reload
- `POST /admin/connectors/:name/toggle` — toggle a connector

Webhook endpoints live under `/webhooks/`.

## Quick start

```sh
cp gluebox.example.toml gluebox.toml
# fill in credentials
gluebox          # start the daemon
gluebox tui      # open the TUI dashboard (in another terminal)
```

Config path defaults to `gluebox.toml` in the working directory. Override with `GLUEBOX_CONFIG=/path/to/config.toml`.

## CLI reference

| Command | Description |
|---|---|
| `gluebox` | Run the daemon |
| `gluebox tui` | Connect to a running daemon and show the dashboard |
| `gluebox status` | Print current connector and power state as JSON |
| `gluebox reload` | Trigger a config reload on the running daemon |
| `gluebox toggle <name>` | Toggle a connector by name |

`status`, `reload`, and `toggle` talk to the admin API at `http://127.0.0.1:8990`. If `listen_addr` is customised the commands will need to be pointed at the right address manually (or via the admin API directly).

## Config reference

See `gluebox.example.toml` for full examples.

| Section | Controls |
|---|---|
| *(root)* | `listen_addr`, `notify_secret`, `socket_path` |
| `[turso]` | libsql/Turso database URL, auth token, optional local replica path |
| `[power]` | LIF power manager thresholds and timing |
| `[linear]` | Linear API key, webhook secret, team ID |
| `[anytype]` | Anytype HTTP API URL, API key, space ID |
| `[matrix]` | Matrix homeserver, access token, room IDs, optional bot credentials |
| `[documenso]` | Documenso API URL, API key, webhook secret |
| `[opencode]` | OpenCode API key (enables the OpenClaw chatbot) |
| `[github]` | GitHub token, repo, webhook secret |

All connector sections are optional. Omitting a section disables that connector entirely.

## Adding a new connector

1. Implement `Connector` for your type in `src/connectors/<name>.rs`.
2. Add a config struct to `src/config.rs` and add an `Option<YourConfig>` field to `Config`.
3. Expose the module in `src/connectors/mod.rs`.
4. Register the connector in `main.rs` alongside the existing connector registrations.
5. Add the corresponding TOML section to `gluebox.toml`.

## Power management

The power manager is modelled on a leaky integrate-and-fire (LIF) neuron:

- Each activity event fires a spike that raises the membrane potential by `spike_weight`.
- A periodic tick decays the potential by `decay_rate`, floor zero.
- When potential reaches `threshold` the daemon enters **Active** state and all suspended connectors are resumed.
- Once below `threshold`, the daemon stays Active for at least `min_active_secs` before returning to **Resting** and suspending connectors.
- `tick_interval_secs` controls how often the decay tick fires.

All parameters are configurable in `[power]` and hot-reloadable.

## Building

```sh
cargo build --release
```

Nix package:

```sh
nix build .#gluebox
```
