# 📋 ClipLAN

Passwordless LAN clipboard & file sharing. Copy on your PC, open ClipLAN on
your phone, paste. No accounts, no cloud, no internet connection required —
everything stays on your local network.

```
Copy on PC → open ClipLAN on phone → paste/copy/download instantly.
```

## Status

This is a working **V1** implementation (see [Roadmap](#roadmap) below for
what's deliberately deferred to V1.1/V2). It was written and reviewed
carefully, but **could not be compiled or run in the environment that
produced it** — that sandbox has no Rust toolchain and no network access to
fetch crates. Before you trust it, run the standard checks yourself:

```bash
cargo check
cargo test
cargo build --release
cargo run --release
```

Please treat this as a solid first draft that needs a normal edit/compile/
fix loop, not as pre-verified working code. If `cargo check` turns up
issues, they're most likely small (a type mismatch, an import) rather than
structural — the architecture and logic have been designed and reviewed
end-to-end.

## Quick start

```bash
# 1. Build
cargo build --release

# 2. Run on your PC
./target/release/cliplan
```

You'll see something like:

```
╭──────────────────────────────────╮
│           📋 ClipLAN              │
╰──────────────────────────────────╯

Status:       ● Running
Port:         8787
Address:      192.168.1.25

Web:
http://192.168.1.25:8787

Pairing code (enter this on the connecting device): 483920
[QR code]
```

On your phone (same Wi-Fi): scan the QR code, or open the address shown and
enter the pairing code plus a name for your phone. Back on the PC, approve
the request:

```bash
cliplan pair
```

Once approved, the phone is paired — copy something on the PC and it shows
up on the phone within moments, and vice versa. Drag a file onto the
dashboard on either device to send it to the other.

## CLI

```
cliplan                 Run the server (default)
cliplan status           Show server status
cliplan pair             Review / approve / reject pending pairing requests
cliplan devices          Show pending pairing requests (paired-device list is in the web UI)
cliplan history           Hint for viewing clipboard history
cliplan config            Print the resolved configuration

Flags (apply to the default run command):
  --port <PORT>
  --directory <DIR>
  --no-discovery
  --no-clipboard
  --config <PATH>
```

Configuration lives at `~/.cliplan/config.toml`:

```toml
port = 8787
shared_directory = "./files"
max_upload_size = 1073741824
clipboard_sync = true
clipboard_history = true
discovery = true
```

Data lives under `~/.cliplan/`:

```
~/.cliplan/
├── cliplan.db      # sqlite metadata only — devices, clipboard entries, transfer records
├── files/          # actual uploaded file bytes
└── config.toml
```

## Architecture

```
                       LAN
                        │
       ┌────────────────┼────────────────┐
       │                │                │
       ▼                ▼                ▼
     PC App           Phone           Laptop
   (web dashboard,  (web dashboard   (web dashboard
    served by the    over LAN IP)     over LAN IP)
    same binary)
       │                │                │
       └────────────────┼────────────────┘
                        │
                        ▼
                ┌───────────────┐
                │ ClipLAN Rust  │
                │    Server     │
                ├───────────────┤
                │ HTTP API      │
                │ WebSocket     │
                │ Discovery     │
                │ Clipboard     │
                │ File Transfer │
                └───────────────┘
```

One Rust binary (Axum + Tokio) serves both the REST/WebSocket API *and* the
static web frontend (embedded into the binary at compile time via
`include_str!`, so there's a single file to ship). SQLite stores metadata
only; file bytes live on disk under the configured shared directory.

```
src/
├── main.rs        CLI entry point (thin — delegates to the `cliplan` library)
├── lib.rs         Library root, re-exports every module (also what tests/ link against)
├── config.rs       Config file + CLI-flag loading
├── server.rs        AppState, router assembly, startup banner
├── error.rs        Central AppError -> HTTP response mapping
├── api/            HTTP handlers (clipboard, devices, files, transfers) + auth extractors
├── clipboard/      Content-type detection; optional desktop clipboard watcher (feature "agent")
├── discovery/       UDP broadcast LAN discovery
├── pairing/         Pairing session (code/QR) + device identity (tokens, fingerprints)
├── websocket/       WebSocket upgrade handler + event types
├── storage/         SQLite schema & queries, shared data models
└── transfer/         Upload streaming+hashing, download+Range serving, filename sanitization

web/                Vanilla HTML/CSS/JS frontend (no framework, no build step)
tests/              Integration tests that boot a real server and hit it over HTTP
```

### Pairing & authentication model

ClipLAN has no user accounts or passwords, but it is **not** unauthenticated:

1. The server always keeps one pairing session active, rotating every 4
   minutes. Its 6-digit code is shown on the host's terminal and, if the
   dashboard is opened at `http://localhost:<port>` on the host itself,
   there too. The QR code just encodes the pairing URL (`/pair/<session>`)
   so a phone doesn't have to type an IP address — the code still has to be
   entered separately.
2. A new device visits that URL (or types the address + code manually),
   submits a display name, and gets back a **pending** request. This grants
   nothing yet.
3. Approval requires a request from **the host machine itself**
   (`127.0.0.1`/`::1`, enforced by checking the TCP peer address, not a
   header a client could fake) — via `cliplan pair` or the dashboard opened
   on the host. This is the actual trust boundary: anyone on the LAN can
   *ask* to pair, only someone with local access to the host can *approve*.
4. On approval the server mints a random 256-bit token, stores only its
   SHA-256 hash, and hands the plaintext token to the device exactly once.
   Every subsequent request carries it as `Authorization: Bearer <token>`;
   the WebSocket carries it as a `?token=` query parameter since browsers
   can't set custom headers on the handshake.
5. Wrong codes count against a per-session attempt limit (5); exceeding it
   invalidates the session outright rather than letting someone grind
   through all 1,000,000 six-digit codes.

### File transfer integrity

Uploads are streamed straight to disk (never buffered whole in memory)
while being hashed with SHA-256 incrementally. The final hash is what gets
stored and returned — so "transfer complete" always means "hash verified",
not "bytes arrived." Filenames are sanitized to a single safe path
component and files are always written under a UUID-prefixed name inside
the configured shared directory, so a malicious filename (`../../etc/passwd`
and friends) can't be used to write outside it. Downloads support HTTP
`Range` requests, so a client can resume an interrupted download without
ClipLAN needing a bespoke resume protocol.

## Security documentation

- **Transport**: V1 serves plain HTTP, as called for in the spec for local
  development. **This means anyone who can sniff your LAN traffic (e.g. a
  compromised device on the same Wi-Fi, or a malicious access point) can see
  clipboard contents, file contents, and bearer tokens in transit.** The
  architecture doesn't assume HTTP, though: swapping in `axum-server`'s TLS
  support (a self-signed or locally-trusted cert) is a config change, not a
  redesign, and is the first thing to add before using ClipLAN on any
  network you don't fully trust.
- **Trust model**: pairing approval is gated on loopback-only access (see
  above) — being on the same Wi-Fi is enough to *request* pairing, never
  enough to *get* paired.
- **Tokens**: only SHA-256 hashes are stored; the one-time plaintext token
  handoff happens over a normal API response, not embedded in a URL or QR
  code.
- **Filesystem**: uploaded filenames are sanitized (path separators and
  traversal sequences stripped) and files are stored under
  UUID-prefixed names inside a fixed directory, with a canonicalization
  check on download as defense in depth. There is no endpoint that opens an
  arbitrary path from client input.
- **Size limits**: both an outer `axum::extract::DefaultBodyLimit` and an
  inner streaming check (which stops and deletes the partial file) enforce
  `max_upload_size`; clipboard entries are capped at 256 KiB.
- **Logging**: `tracing` logs device names, filenames, sizes, and hashes —
  never clipboard *contents*, tokens, or keys.
- **What's not done yet**: rate limiting is only implemented for pairing
  code guesses, not general API traffic; there's no per-IP throttle on, say,
  repeated failed-auth requests. See Roadmap.

## Testing

```bash
cargo test
```

`tests/` boots a real ClipLAN instance per test (ephemeral port, throwaway
temp directory) and drives it over actual HTTP with `reqwest`, plus a few
`#[cfg(test)]` unit tests inline in `src/` for pure functions (filename
sanitization, content-type detection).

- `tests/pairing.rs` — pairing succeeds and grants a working token; wrong
  codes are rejected; unapproved/rejected requests never yield a token.
- `tests/api.rs` — auth is required on protected endpoints; clipboard
  create/list/delete roundtrip; device rename.
- `tests/transfer.rs` — upload → download roundtrip byte-for-byte with
  matching SHA-256; oversized uploads rejected; deleting a transfer removes
  the file from disk.
- `tests/security.rs` — path-traversal filenames get sanitized; invalid/
  missing bearer tokens rejected; unknown transfer IDs 404 instead of
  leaking a stack trace; oversized clipboard content rejected.

## Desktop clipboard agent (optional)

Section 5's "Mode B" — watching the OS clipboard in the background — is
implemented behind a Cargo feature so the default build doesn't pull in
platform clipboard bindings:

```bash
cargo build --release --features agent
```

## Roadmap

**V1 (this release)**
Rust server, web interface, QR + code pairing, device pairing/approval,
text clipboard sharing, file upload/download with SHA-256 verification,
WebSocket live updates, SQLite metadata, transfer history, responsive
mobile UI.

**V1.1**
Desktop clipboard monitoring (implemented behind `--features agent`; needs
real-device testing across platforms), LAN discovery (implemented; needs
real-network testing across router/AP configurations), multiple
simultaneous transfers (the server already handles concurrent uploads —
needs frontend polish for a nicer multi-transfer UI), transfer
cancellation (client-side abort is wired up; no server-side partial-upload
resume yet), richer device management.

**V2**
Resumable uploads (downloads already support `Range`; uploads restarting
from the last byte after a dropped connection is not yet implemented),
end-to-end encryption, native desktop tray application, native mobile
apps, background sync.

## License

MIT — see [LICENSE](LICENSE).
