# Gluebox

Gluebox is the integration backbone for Stonkwatch. It exists because we use four tools that don't talk to each other: Linear for project management, Anytype for specs and knowledge, Matrix for encrypted team chat, and Documenso for document signing. When someone files an issue in Linear, the rest of the team shouldn't have to manually copy that into a spec doc, paste a link into chat, and remember to update everything when the status changes. Gluebox handles all of that automatically.

It runs as a single Rust binary that receives webhooks from Linear and Documenso, maintains bidirectional sync with Anytype, posts notifications to an encrypted Matrix room, and hosts an AI-powered chatbot called OpenClaw that lives in that same room.

## The problem

Stonkwatch's workflow spans multiple tools by necessity. Linear is good at issue tracking but knows nothing about our specs. Anytype is good at structured knowledge but has no concept of a project board. Matrix is where actual conversations happen but has no integration with either. Documenso handles contracts and signatures but those completions and rejections need to be visible to the rest of the system.

Without gluebox, every state change requires someone to manually propagate information across tools. An issue gets shipped in Linear? Someone has to update the spec in Anytype and tell the team in Matrix. A contract gets rejected in Documenso? Someone has to file a comment on the related Linear issue and update the Anytype record. This falls apart quickly, and things get missed.

## How it works

Gluebox is a webhook receiver and event router. When something happens in a connected service, gluebox receives the event, verifies its authenticity, and triggers the appropriate actions across other services.

### Linear to Anytype sync

When a Linear issue is created with a "spec" label, gluebox creates a corresponding Spec object in Anytype with the issue's title and description, stores the mapping in its local SQLite database, writes the Anytype object ID back into the Linear issue for cross-referencing, and notifies the Matrix room. When the issue is updated, the Anytype spec is patched to match. When it ships, the spec is marked as shipped and the team gets notified.

### Documenso to Anytype sync

When a document is completed in Documenso (all parties have signed), gluebox creates or updates a Contract object in Anytype with party details and completion date, then notifies Matrix. If a document is rejected, gluebox records the rejection reasons in Anytype, updates the mapping status, and if there's a linked Linear issue, adds a comment about the rejection.

### Webhook verification

Linear webhooks are verified using HMAC-SHA256 with constant-time comparison and a 60-second replay window. Documenso webhooks use a shared secret, also compared in constant time. Invalid signatures get a 401.

### OpenClaw

OpenClaw is an AI chatbot that lives in the Stonkwatch Matrix room. It responds to `!bot` messages and can draft technical specs, write Architecture Decision Records, create Linear issues from natural language descriptions, or just have a conversation. Intent classification uses fast keyword matching first, falling back to AI classification via the OpenCode API. It operates over E2EE, with cross-signing bootstrapped on login and crypto state persisted in SQLite.

## Deployment

Gluebox runs on a NixOS VPS alongside a self-hosted Anytype server. The entire system is defined declaratively in a Nix flake and deployed via deploy-rs from GitHub Actions on push to main.

The server runs five services:

- **Gluebox** itself, listening on `127.0.0.1:8990`, reading config from `/etc/gluebox/gluebox.toml`
- **any-sync-bundle**, a self-hosted Anytype sync server that packages all server-side components into a single Go binary
- **MongoDB**, required by any-sync-bundle for metadata storage (replica set, single node)
- **Valkey** (Redis fork) with the Bloom filter module, required by any-sync-bundle for caching
- **Tailscale**, providing private networking. The firewall only opens port 22 publicly; everything else is accessible only over the Tailscale network via Funnel

Gluebox's own persistence is just SQLite. MongoDB, Valkey, and the Bloom filter module are all dependencies of any-sync-bundle, not gluebox.

The Anytype desktop client connects to the self-hosted server over Tailscale, so specs and contracts created by gluebox appear in Anytype's UI like any other object.

## Configuration

Gluebox reads its config from the path in `$GLUEBOX_CONFIG` (defaults to `gluebox.toml` in the working directory). See `gluebox.example.toml` for the full structure. The config contains API keys and webhook secrets for each integration, the Matrix bot credentials, and the SQLite database path.

The `opencode` section is optional. If absent, OpenClaw is disabled. The `matrix.bot_username` and `bot_password` fields are also optional. Without them, the E2EE bot and encrypted notifications are disabled.

## Building

```
cargo build --release
```

The flake also provides a Nix package:

```
nix build .#gluebox
```
