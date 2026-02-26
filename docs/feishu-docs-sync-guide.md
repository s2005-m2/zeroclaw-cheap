# Feishu Docs Bidirectional Config Sync

Sync your local ZeroClaw workspace files (config, identity, soul) with a Feishu document. Changes flow both ways: edit locally and the document updates, edit the document and local files update.

This feature requires the `feishu-docs-sync` compile-time feature flag.

## Overview

The docs sync engine watches a set of local files and a remote Feishu document. When either side changes, the engine propagates the diff to the other side. The document uses a simple code-block format to hold multiple files:

```
=== config.toml ===
<file content>
=== end ===

=== IDENTITY.md ===
<file content>
=== end ===
```

Local file changes are debounced (500ms) before pushing to Feishu. Remote changes are polled on a configurable interval.

## Prerequisites

1. Build ZeroClaw with the feature flag:

   ```bash
   cargo build --release --features feishu-docs-sync
   ```

2. A Feishu (Lark) custom app with the required permissions (see next section).

3. The app's `app_id` and `app_secret` available in your environment or config.

## Required Feishu App Permissions

Create a custom app in the [Feishu Open Platform console](https://open.feishu.cn/app) and grant these scopes:

| Scope | Purpose |
|---|---|
| `docx:document` | Read and write document content |
| `docx:document:readonly` | Read document content (fallback/read-only flows) |
| `drive:drive` | Access Drive files and metadata |
| `im:message` | Receive message events (for event subscriptions) |
| `im:message:send_as_bot` | Send sync notifications as the bot |

After adding scopes, publish a new app version and wait for tenant admin approval if your org requires it.

## Event Subscription Setup

To get near-real-time remote-to-local sync, subscribe to the `drive.file.edit_v1` event. This fires whenever someone edits the synced document.

Two delivery modes are available:

### WebSocket Long Connection (recommended)

In the Feishu Open Platform console, go to **Event Subscriptions** and select **WebSocket** as the delivery method. This avoids exposing a public endpoint. The Feishu SDK maintains a persistent connection and pushes events to your app.

### Webhook Callback

If you prefer HTTP callbacks, configure a callback URL (for example `https://your-host/feishu/events`) and set the verification token. Your server must respond to the URL verification challenge on first registration.

In both cases, subscribe to:

- Event type: `drive.file.edit_v1`
- Event version: v2.0 (recommended)

When the event fires, ZeroClaw pulls the latest document content and runs the remote-to-local sync pipeline.

## Configuration

Add a `[docs_sync]` section to `~/.zeroclaw/config.toml`:

```toml
[docs_sync]
# Enable the sync engine. Default: false.
enabled = true

# Feishu document ID to sync with.
# Find this in the document URL: https://your-org.feishu.cn/docx/<document_id>
# Leave empty if using auto_create_doc.
document_id = "doxcnXXXXXXXXXXXXXX"

# Files to sync (relative to workspace). Default list shown below.
sync_files = ["config.toml", "IDENTITY.md", "SOUL.md", "USER.md", "AGENTS.md"]

# How to receive remote changes: "polling" (default) or "event" (WebSocket subscription).
# - polling: fetch the document every sync_interval_secs seconds.
# - event: subscribe to drive.file.edit_v1 via Feishu WebSocket long-connection (near-real-time).
remote_mode = "polling"

# Polling interval for remote changes, in seconds. Default: 60.
# Used as the polling interval when remote_mode = "polling".
# When remote_mode = "event", this is used as a fallback full-sync interval.
sync_interval_secs = 60

# Automatically create a new Feishu document if document_id is empty.
# The created document ID is logged but not written back to config.
# Default: false.
auto_create_doc = false

# Optional: Feishu App ID for event subscription.
# Falls back to [channels_config.feishu].app_id if not set.
# app_id = "cli_xxxxxxxxxxxx"

# Optional: Feishu App Secret for event subscription.
# Falls back to [channels_config.feishu].app_secret if not set.
# app_secret = "xxxxxxxxxxxxxxxxxxxxxxxx"

# Optional: Encrypt key for WebSocket event decryption (from Feishu console).
# encrypt_key = "your_encrypt_key"
```

### Field Reference

| Field | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Master switch for the sync engine |
| `document_id` | string | `""` | Feishu document ID to sync with |
| `sync_files` | vec of strings | `["config.toml", "IDENTITY.md", "SOUL.md", "USER.md", "AGENTS.md"]` | Local files included in sync |
| `remote_mode` | string | `"polling"` | How to receive remote changes: `"polling"` or `"event"` |
| `sync_interval_secs` | u64 | `60` | Polling interval (seconds); also fallback full-sync interval in event mode |
| `auto_create_doc` | bool | `false` | Create a new document when `document_id` is empty |
| `app_id` | string (optional) | `None` | Feishu App ID for event subscription; falls back to channel config |
| `app_secret` | string (optional) | `None` | Feishu App Secret for event subscription; falls back to channel config |
| `encrypt_key` | string (optional) | `None` | Encrypt key for WebSocket event decryption |

Feishu app credentials (`app_id` and `app_secret`) can be provided directly in `[docs_sync]` or inherited from `[channels_config.feishu]`. When using `remote_mode = "event"`, at least one source of credentials is required.

## Security Considerations

The sync engine enforces a hard boundary on sensitive config sections. When pulling remote changes to `config.toml`, the engine scans for forbidden TOML section headers. If any are found, the entire remote sync is rejected.

Protected sections:

- `[security]`
- `[gateway]`
- `[autonomy]`

This means nobody can change your security policy, gateway binding, or autonomy level through a Feishu document edit. The rejection is all-or-nothing: if a forbidden section appears anywhere in the remote config content, no files from that sync cycle are written.

Additional safety measures:

- Symlink targets are never written to. If a sync file resolves to a symlink, it's skipped with a warning.
- Only files listed in `sync_files` are written. Unlisted files present in the remote document are ignored.
- The sync engine uses the same `tenant_access_token` caching as the Lark channel, with proactive refresh 120 seconds before expiry.

## Troubleshooting

### Rate Limits

The Feishu `batch_update_blocks` API enforces a rate limit of roughly 3 requests per second. The sync client respects this internally by spacing write calls at least 334ms apart. If you see `429` or throttling errors in logs, check whether other integrations sharing the same app are consuming quota.

### Permission Errors

Common causes:

- App scopes not approved by tenant admin. Check the app's status in the Feishu Open Platform console.
- Document not shared with the app. The app's bot identity needs at least edit access to the target document.
- `app_id` / `app_secret` mismatch. Verify credentials match the app that holds the granted scopes.

If you see `Feishu tenant_access_token failed` in logs, the credentials are wrong or the app hasn't been published/approved.

### Conflict Resolution

The sync engine uses last-write-wins semantics. There's no merge or three-way diff. If both sides change the same file between sync intervals, the side that syncs last overwrites the other.

To reduce conflicts:

- Keep `sync_interval_secs` low (30-60s) for near-real-time convergence.
- Avoid editing the same file locally and remotely at the same time.
- Use event subscriptions (`drive.file.edit_v1`) so remote changes arrive promptly instead of waiting for the next poll.

## Hook Support

The `on_docs_sync_notify` hook fires after a sync cycle completes, before any notification is sent to channels. It receives four parameters:
- `file_path`: the file that was synced
- `channel`: target notification channel
- `recipient`: notification recipient
- `content`: notification message body
A hook can modify any of these values (for example, rewrite the notification text or redirect it to a different channel) or cancel the notification entirely by returning `HookResult::Cancel`.

Example use cases:
- Suppress noisy notifications for `AGENTS.md` changes.
- Route config change alerts to a dedicated ops channel.
- Append a diff summary to the notification content.

## i18n Follow-Up

Per the project's documentation governance (AGENTS.md §4.1), localized versions of this guide are needed for all supported locales: `zh-CN`, `ja`, `ru`, `fr`, `vi`. This will be handled in a follow-up PR.
