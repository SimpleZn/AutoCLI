# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Project Is

AutoCLI is a Rust CLI tool that turns websites into command-line interfaces. It loads declarative YAML adapter files (embedded at compile time) and executes them as pipelines — fetching APIs, running browser automation, transforming data, and rendering results. There is also a Chrome extension for visual adapter generation.

## Build & Test Commands

```bash
make build            # Debug build
make release          # Release build (current platform)
make test             # cargo test --workspace (unit tests)
make test-adapters    # Integration tests against live adapters
make install          # Install to /usr/local/bin
make release-all      # Cross-compile for macOS/Linux/Windows
make clean            # Clean build artifacts
```

Run a single test:
```bash
cargo test -p autocli-pipeline test_name
```

Integration test script (122 commands):
```bash
./scripts/test-all-commands.sh
```

Chrome extension (TypeScript + Vite):
```bash
cd extension && npm install && npm run build
```

## Workspace Architecture

8 Rust crates with this dependency order:

```
autocli-core         # Data models: CliCommand, IPage trait, Registry, CliError
     ↓
autocli-pipeline     # Executes YAML pipeline steps (fetch, map, filter, evaluate, etc.)
autocli-browser      # Daemon (Axum HTTP+WS on port 19925) + CDP + BrowserBridge
autocli-discovery    # Parses YAML adapters; build.rs embeds all YAML into the binary
autocli-external     # Passthrough execution for external CLIs (gh, docker, kubectl)
autocli-output       # Multi-format rendering: table, JSON, YAML, CSV, Markdown
autocli-ai           # AI features: explore, generate, cascade, synthesize
     ↓
autocli-cli          # main.rs: clap CLI, daemon management, routing, execution
```

**Critical design constraint**: All adapter YAML files are embedded at compile time via `crates/autocli-discovery/build.rs`. There is no file I/O at runtime for adapters — changes to YAML require a rebuild.

## How Execution Works

1. `main.rs` checks if the daemon is running (port 19925); spawns it if needed or if version mismatches.
2. Discovery loads builtin (embedded) + user adapters (`~/.autocli/adapters/`) and external CLI registry.
3. clap builds subcommands dynamically from the registry.
4. On a command match: if `browser: true` in the adapter, `BrowserBridge` connects to the daemon via CDP; otherwise runs a pure HTTP pipeline.
5. The pipeline executor in `autocli-pipeline` iterates steps, evaluating `${{ expr | filter }}` templates via a pest PEG parser.
6. Results are rendered by `autocli-output`.

## Adapter YAML Structure

Adapters live in `adapters/<site>/<name>.yaml`. The pipeline steps are:

| Step | Purpose |
|------|---------|
| `fetch` | HTTP request with URL templating |
| `evaluate` | Execute JS in browser context (requires `browser: true`) |
| `navigate` | Navigate browser to URL |
| `click` / `type` / `wait` / `select` | Browser DOM interactions |
| `map` | Transform array items with `${{ item.field \| filter }}` |
| `filter` | Filter array: `filter: "item.score > 10"` |
| `sort` | Sort by field/order |
| `limit` | Truncate to N items |
| `intercept` | Capture network requests matching a URL pattern |
| `tap` | Bridge to Pinia/Vuex state |
| `download` | Download media/articles |

Template expression filters: `default`, `join`, `upper`, `lower`, `trim`, `truncate`, `replace`, `keys`, `length`, `first`, `last`, `json`, `slugify`, `sanitize`, `ext`, `basename`.

Strategy values: `public` (no auth), `cookie` (browser cookie), `header` (API key), `intercept` (network capture), `ui` (full browser interaction).

User custom adapters go in `~/.autocli/adapters/<site>/<name>.yaml` and are discovered at runtime without recompiling.

## Key Files

- `crates/autocli-cli/src/main.rs` — CLI entry point, daemon lifecycle, routing
- `crates/autocli-cli/src/execution.rs` — `execute_command`, browser vs. non-browser dispatch
- `crates/autocli-pipeline/src/executor.rs` — pipeline step orchestration
- `crates/autocli-pipeline/src/template/` — pest PEG parser for `${{ }}` expressions
- `crates/autocli-pipeline/src/steps/` — one module per step type
- `crates/autocli-browser/src/daemon.rs` — Axum HTTP + WebSocket daemon
- `crates/autocli-browser/src/cdp.rs` — Chrome DevTools Protocol client
- `crates/autocli-core/src/page.rs` — `IPage` async trait (browser abstraction)
- `crates/autocli-discovery/build.rs` — compile-time YAML embedding
- `crates/autocli-external/resources/external-clis.yaml` — external CLI registry

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `AUTOCLI_DAEMON_PORT` | `19925` | Daemon HTTP port |
| `AUTOCLI_BROWSER_COMMAND_TIMEOUT` | `60` | Pipeline timeout in seconds |
| `AUTOCLI_VERBOSE` | — | Enable verbose tracing |
| `RUST_LOG` | — | Standard tracing filter |

## Runtime Config

- `~/.autocli/config.json` — user token, settings, CookieCloud credentials
- `~/.autocli/adapters/` — user custom adapters (runtime, no recompile needed)
- `~/.autocli/external-clis.yaml` — user-registered external CLIs

## CookieCloud Integration

AutoCLI integrates with [CookieCloud](https://github.com/easychen/CookieCloud) to inject browser cookies from a self-hosted server into the Chrome automation window before page navigation. This enables `strategy: cookie` adapters to authenticate without manual login.

### Architecture

```
Real Chrome + CookieCloud extension
  → syncs cookies → CookieCloud server (self-hosted, AES-encrypted)
       ↓  autocli cookies sync
  ~/.autocli/config.json  { cookiecloud: { server_url, uuid, password } }
       ↓  on each browser command
  fetch cookies for domain  →  DaemonCommand "set-cookies"
       →  WebSocket  →  Chrome extension  →  chrome.cookies.set()
       →  automation window navigates with session cookies already present
```

### Key design decisions

- **httpOnly cookies**: `chrome.cookies.set()` (via `set-cookies` daemon action) is used instead of `document.cookie` JS injection, which cannot set httpOnly cookies like `SESSDATA`.
- **Injection timing**: cookies are injected *before* `navigate` so the very first HTTP request carries the session.
- **Domain matching**: fuzzy — `weread.qq.com` matches the `qq.com` CookieCloud key; `www.bilibili.com` matches `.bilibili.com`.
- **Timeout**: reqwest uses `connect_timeout(10s)` + `timeout(120s)` because large encrypted payloads (~400 KB) transfer slowly through proxies.
- **Decryption**: supports both CookieCloud modes — legacy (AES-256-CBC, CryptoJS EVP_BytesToKey + salted header) and fixed-IV (AES-128-CBC, zero IV).

### Relevant files

- `crates/autocli-ai/src/cookiecloud.rs` — HTTP fetch + AES decrypt + domain filter
- `crates/autocli-ai/src/config.rs` — `CookieCloudConfig` struct
- `crates/autocli-browser/src/page.rs` — `set_cookies()` uses `set-cookies` daemon action
- `crates/autocli-browser/src/types.rs` — `DaemonCommand.cookies` field
- `extension/src/background.ts` — `handleSetCookies()` calls `chrome.cookies.set()`
- `extension/src/protocol.ts` — `set-cookies` action + `cookies` field in `Command`
- `crates/autocli-cli/src/execution.rs` — `inject_cookiecloud_cookies()` called before navigate
- `crates/autocli-cli/src/main.rs` — `cookies setup/sync/list` subcommands
- `crates/autocli-cli/src/commands/doctor.rs` — CookieCloud health check

### User workflow

```bash
# 1. Setup (one time)
autocli cookies setup          # prompts for server URL, UUID, password

# 2. Sync (after logging into sites in real Chrome + CookieCloud extension sync)
autocli cookies sync           # fetches & decrypts from CookieCloud server

# 3. List (verify what was synced)
autocli cookies list           # all domains
autocli cookies list bilibili.com   # specific domain

# 4. Use normally — cookies are injected automatically
autocli bilibili me
autocli weread shelf

# 5. Diagnose
autocli doctor                 # shows CookieCloud status + cookie count
```

### Chrome extension installation

```bash
cd extension && npm install && npm run build
```

Then in Chrome → `chrome://extensions` → Developer mode → Load unpacked → select the `extension/` directory. After any TypeScript change, re-run `npm run build` and click reload on the extension card.

### Crypto crates (autocli-ai/Cargo.toml)

```toml
md5 = "0.7"
aes = "0.8"
cbc = { version = "0.1", features = ["alloc"] }
base64 = "0.22"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "gzip"] }
```

The `gzip` feature is required — CookieCloud servers return gzip-compressed responses.
