# Deployer Playbook

Audience: the **deployer / operator-admin**. Someone setting up `brain-mcp`
on a fresh VM, configuring MCP clients, scheduling backups, and
troubleshooting production issues.

**This file covers Shape C (systemd bare-metal) in depth, and Shape B
(Docker compose) in §0 below.** If you don't yet know which shape a host
is running, read `references/deployment-shapes.md` first — it has the
detection heuristic and is the shape-agnostic source of truth; this file
is the deep dive once you know you're on Shape C (or want the Docker
compose quick-start in §0).

Sister playbooks:

- `operator.md` — day-to-day memory and knowledge work
- `developer.md` — contributing to `brain-mcp` source

When this playbook and the brain-mcp repo disagree, the repo wins
(SKILL.md rule).

---

## 0. Shape B: Docker compose quick-start

Skip to §1 if you're deploying Shape C (systemd bare-metal) instead.

This is the shape actually running in production as of 2026-07-19 (see
that instance's `entities/brain-instance` page) — a single-service
Docker compose stack, published loopback-only, config bind-mounted
read-only for hardening. Full command reference (start/restart/upgrade/
the EROFS gotcha for `wiki_spaces_create`) is in
`references/deployment-shapes.md` § Shape B — this section covers the
one-time setup only.

### Setup

```bash
mkdir -p ./config ./secrets ./data ./backups
cp examples/config.docker.toml ./config/config.toml
(umask 077; printf '%s' "$(openssl rand -hex 32)" > ./secrets/bootstrap_secret.txt)
docker compose up -d --build
curl -sf http://127.0.0.1:8080/ready
```

The compose file (`docker-compose.yml` in the repo root) publishes only
`127.0.0.1:8080:8080` by default. **Widening that publish to a VM's
public interface requires a TLS-terminating reverse proxy in front —
the `/mcp` HTTP path itself has no per-caller auth** (see anti-pattern
#10 in `anti-patterns.md`). This applies identically whether the Docker
host is a laptop or a cloud VM; Shape B does not become "more secure"
just because it's running on a VM instead of a laptop.

### On a VM instead of a laptop

Nothing above changes — `docker compose up -d` behaves the same on an
Oracle/AWS/DigitalOcean VM as on a local Docker Desktop host. The only
addition on a VM is the network boundary: keep the compose publish
loopback-only and reach it via Tailscale/Cloudflare Tunnel/SSH port-
forward, exactly as §2.3–2.4 below describe for Shape C's MCP port. Do
not open the Docker-published port directly in a cloud firewall.

### Backups on a VM

`./backups` is a bind mount — on a VM, treat it exactly like §5 below
describes for Shape C (encrypted, offsite-copied, tested restores), just
substitute the container's backup path for the systemd instance's.

---

## 1. Deployment Topology (Shape C: systemd bare-metal)

Recapped from `BLUEPRINT.md` §9.

### Host spec

- **Shape:** Oracle Cloud Always Free `VM.Standard.A1.Flex` (Ampere ARM)
- **Resources:** 4 OCPU + 24 GB RAM + 200 GB block storage
- **OS:** Ubuntu 24.04 LTS ARM64
- **Single host, no Docker required.** systemd manages every service.
  Docker is supported (see `docs/guides/deploy-docker.md`) but not
  recommended on Always Free — it adds a layer without solving a problem
  on a single-host deployment.

### Service layout

| Service | Resource | Port | Auto-restart | Status |
|---|---|---|---|---|
| `llm-wiki` engine (BM25 + git + petgraph) | ~300 MB RAM | 47778 (HTTP), stdio | systemd | Stable |
| brain-mcp gateway (MCP wrapper) | ~200 MB RAM | 8765 | systemd | Stable |
| Qdrant (vector index) | ~2 GB @ 10K vectors | 6333 (gRPC), 6334 (HTTP) | systemd | Future |
| Embedding service (FastEmbed / Infinity, BGE-M3) | ~3 GB RAM | 8001 | systemd | Future |
| Reranker (BGE-reranker-v2-m3) | ~2 GB RAM | 8002 | systemd | Future |
| OS + buffer | ~5 GB | — | — | — |
| **Total (full stack)** | **~12 GB used / 24 GB available** | | | |

The headroom (about 12 GB) is reserved for peak load and index rebuilds.
Do not size the host down to fit only the stable services.

### Topology diagram

```
                   ┌─────────────────────────────────────────┐
                   │  Operator device (laptop / workstation) │
                   │                                         │
                   │  Claude Desktop  Claude Code  Codex CLI │
                   │  Cursor  Windsurf  Zed  VS Code         │
                   └──────────────┬──────────────────────────┘
                                  │
                                  │ Tailscale / Cloudflare Tunnel
                                  │ (no public ports)
                                  │
                                  │ MCP over stdio (local) or HTTP (remote)
                                  v
   ┌──────────────────────────────────────────────────────────────┐
   │  Oracle Cloud Always Free — VM.Standard.A1.Flex (ARM Ampere) │
   │  Ubuntu 24.04 LTS ARM64                                     │
   │                                                             │
   │   systemd services                                          │
   │   ┌─────────────────────────┐   ┌────────────────────────┐  │
   │   │ brain-mcp gateway (:8765│   │ llm-wiki engine        │  │
   │   │  or via reverse proxy)  │──>│ (:47778 HTTP + stdio)  │  │
   │   └─────────────────────────┘   └───────────┬────────────┘  │
   │                                            │                │
   │           ┌────────────────────────────────┼──────────┐     │
   │           v                  v             v          v     │
   │   ┌──────────────┐  ┌──────────────┐ ┌──────────┐ ┌──────┐ │
   │   │ Qdrant       │  │ Embedding    │ │ Reranker │ │ Web  │ │
   │   │ (:6333/6334) │  │ (:8001)      │ │ (:8002)  │ │ UI   │ │
   │   │ [future]     │  │ [future]     │ │ [future] │ │ :1414│ │
   │   └──────────────┘  └──────────────┘ └──────────┘ └──────┘ │
   │                                                             │
   │   Canonical filesystem: ~/wikis/brain (git repo)           │
   │     profile/ concepts/ entities/ sources/ projects/         │
   │     decisions/ procedural/ schemas/ inbox/ raw/             │
   │     site/  (generated Hugo web UI mirror)                   │
   │     .git/  (authored document history)                      │
   │                                                             │
   │   Daily: git push → private GitHub/Gitea                    │
   │   Weekly (future): Qdrant snapshot → Oracle Object Storage  │
   └──────────────────────────────────────────────────────────────┘
```

The VM is the single source of truth. There is no multi-master sync in
v1 (BLUEPRINT §0.2: single-operator design).

---

## 2. Initial Provisioning

A cold-start walk-through for a fresh Ubuntu 24.04 ARM64 VM. Run all
commands as your operator user (not root); prefix `sudo` only where
shown.

### 2.1 Provision the Oracle VM

In the Oracle Cloud console:

1. Create a compute instance with shape `VM.Standard.A1.Flex`.
2. Set 4 OCPU and 24 GB RAM (max Always Free allowance).
3. Attach a 200 GB block storage volume.
4. Image: `Canonical-Ubuntu-24.04-aarch64-*`.
5. Add your SSH public key.
6. After launch, configure the VCN: keep public ingress closed. Use
   Tailscale or Cloudflare Tunnel for access (see §6).

Do **not** open MCP ports in the Oracle security list. The whole point
of this deployment is private access only.

### 2.2 Install Rust toolchain (only if building from source)

`install.sh` downloads a prebuilt binary, so this step is only needed
when you intend to `cargo install --path .` or build debug binaries.

```bash
# Install rustup + stable toolchain + ARM64 target
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# Confirm
rustc --version
cargo --version

# Add the ARM64 Linux target (already native on Ampere, but explicit is safer)
rustup target add aarch64-unknown-linux-gnu
```

### 2.3 Install the binary

**Option A — `install.sh` (recommended).** Detects platform, downloads
the latest published GitHub Release, installs `llm-wiki`, verifies `git`,
and tries to install Hugo Extended for the web UI.

```bash
curl -fsSL https://raw.githubusercontent.com/hataichanokpan-dev/brain-mcp/main/install.sh | bash
```

Custom install directory or skip Hugo:

```bash
LLM_WIKI_INSTALL_DIR=~/.local/bin \
curl -fsSL https://raw.githubusercontent.com/hataichanokpan-dev/brain-mcp/main/install.sh | bash

# Skip automatic Hugo install
LLM_WIKI_INSTALL_HUGO=0 \
curl -fsSL https://raw.githubusercontent.com/hataichanokpan-dev/brain-mcp/main/install.sh | bash
```

For a private repository, clone locally and run the script from there,
or export `GITHUB_TOKEN` / `GH_TOKEN` first so release lookup and
download can authenticate.

The installer installs the **latest published GitHub Release**, not
merely the latest pushed tag. If a release workflow is still building,
wait for assets or build from source (Option B).

**Option B — From source.** Requires Rust 1.95+.

```bash
git clone https://github.com/hataichanokpan-dev/brain-mcp.git
cd brain-mcp
cargo install --path .
```

The release profile uses LTO + `codegen-units=1` for a small, stripped
binary, so the first build is slow (10–15 min on Ampere).

### 2.4 Verify

```bash
llm-wiki --version
which llm-wiki
```

If the binary is installed but not on `PATH`:

```bash
# Add to ~/.bashrc or ~/.zshrc
export PATH="/usr/local/bin:$PATH"
```

### 2.5 Create the brain wiki space

```bash
mkdir -p ~/wikis
llm-wiki spaces create ~/wikis/brain --name brain --set-default
llm-wiki spaces list
```

The first wiki created becomes the default. Confirm `brain` is marked
`*` in the listing.

### 2.6 First index rebuild

```bash
llm-wiki index rebuild --wiki brain
```

This populates the Tantivy BM25 index. Run it after any bulk content
addition or schema change.

### 2.7 First serve test

Stdio (the simplest path; Claude Desktop, Codex CLI, Cursor, etc. all
speak stdio MCP):

```bash
llm-wiki serve
```

You should see startup logs ending in "MCP server ready" (or similar).
Press `Ctrl+C` to stop.

HTTP (for remote clients or for Zed ACP, which needs stdio exclusively):

```bash
llm-wiki serve --http :47778
```

Probe it from another shell on the same host:

```bash
curl -sf http://127.0.0.1:47778/health || echo "health probe failed"
```

Do **not** leave the server attached to your SSH session in production.
Move to §3 to install a systemd unit.

---

## 3. systemd Service Setup

Production runs `llm-wiki serve` under systemd so it survives reboots,
auto-restarts on crash, and has structured logs via `journalctl`.

### 3.1 Unit file template

Create `/etc/systemd/system/brain-mcp.service` as root:

```bash
sudo tee /etc/systemd/system/brain-mcp.service >/dev/null <<'EOF'
[Unit]
Description=brain-mcp / llm-wiki knowledge server
Documentation=https://github.com/hataichanokpan-dev/brain-mcp
After=network-online.target
Wants=network-online.target

[Service]
Type=simple

User=brain
Group=brain

WorkingDirectory=/home/brain

# Use an absolute binary path. If systemd cannot exec it, you get
# status=203/EXEC — see the troubleshooting section.
ExecStart=/usr/local/bin/llm-wiki serve \
    --http :47778 \
    --watch \
    --web \
    --web-port 1414 \
    --web-bind 0.0.0.0

# Graceful shutdown: SIGTERM, then SIGKILL after 30s
KillSignal=SIGINT
TimeoutStopSec=30

# Restart policy
Restart=on-failure
RestartSec=5

# Hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full
ProtectHome=read-only
ReadWritePaths=/home/brain

# Environment
Environment=RUST_LOG=info
Environment=RUST_BACKTRACE=1
Environment=LLM_WIKI_CONFIG=/home/brain/.llm-wiki/config.toml

# Resource limits (tune for your host)
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
EOF
```

Notes on the template:

- `User=brain` — create that user first: `sudo useradd -m -s /bin/bash brain`.
- `WorkingDirectory` must be writable by `brain` because `llm-wiki`
  writes the search index, snapshots, and (in vnext) the event ledger
  under the wiki tree or `~/.llm-wiki/`.
- `ProtectSystem=full` makes `/usr`, `/boot`, `/etc` read-only for the
  service. `ReadWritePaths` whitelists `/home/brain`. Adjust if your
  wiki lives elsewhere.
- `KillSignal=SIGINT` so `llm-wiki` receives Ctrl+C-style shutdown and
  can flush the index and event ledger cleanly.

### 3.2 Reload, enable, start

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now brain-mcp
sudo systemctl status brain-mcp --no-pager
```

`enable --now` both registers the unit for boot and starts it
immediately. Verify `Active: active (running)`.

### 3.3 Log inspection

```bash
# Tail recent logs
sudo journalctl -u brain-mcp -n 200 --no-pager

# Live follow
sudo journalctl -u brain-mcp -f

# Filter errors since boot
sudo journalctl -u brain-mcp -b -p err --no-pager

# Service status with last 10 log lines
sudo systemctl status brain-mcp --no-pager
```

For JSON-structured logs (aggregator-friendly), set the log format in
the global config:

```bash
llm-wiki config set logging.log_format json --global
sudo systemctl restart brain-mcp
```

### 3.4 Common failure: `status=203/EXEC`

`status=203/EXEC` means systemd could not execute `ExecStart` at all —
the binary never started. Diagnose in this order:

```bash
# 1. Does the binary exist at the configured path?
ls -l /usr/local/bin/llm-wiki

# 2. Is it on PATH for you? (does NOT help systemd, but confirms install)
command -v llm-wiki
/usr/local/bin/llm-wiki --version

# 3. Is it executable?
sudo chmod +x /usr/local/bin/llm-wiki

# 4. Is it the right architecture? Ampere = aarch64. An x86_64 binary
#    will exec-fail on ARM with misleading errors.
file /usr/local/bin/llm-wiki
# Expected: ... ELF 64-bit LSB executable, ARM aarch64 ...

# 5. Is SELinux blocking it? (Ubuntu default is no SELinux, but RHEL-
#    derived hosts have it.)
sudo restorecon -v /usr/local/bin/llm-wiki || true

# 6. Reload + restart after any fix.
sudo systemctl daemon-reload
sudo systemctl restart brain-mcp
sudo systemctl status brain-mcp --no-pager
```

If the binary lives in `~/.cargo/bin` or `~/.local/bin`, either move it
to `/usr/local/bin/` (`sudo install -m 0755 ~/.local/bin/llm-wiki
/usr/local/bin/`) or change `ExecStart` to the absolute path. Relative
paths and shell-style `~` expansion do **not** work in unit files.

---

## 4. MCP Client Configuration

Each MCP client has its own config file. Use the **absolute** binary
path (`/usr/local/bin/llm-wiki` on the server, or the full
`/Users/.../llm-wiki` on macOS) when the client cannot find `llm-wiki`
on `PATH`. Claude Desktop in particular spawns the server with a
minimal environment and ignores your shell `PATH`.

### 4.1 Claude Desktop (macOS)

File: `~/Library/Application Support/Claude/claude_desktop_config.json`

Local stdio (binary on the same machine as Claude Desktop):

```json
{
  "mcpServers": {
    "brain": {
      "command": "/usr/local/bin/llm-wiki",
      "args": ["serve"]
    }
  }
}
```

Remote HTTP via the `mcp-remote` bridge (when the server runs on the
Oracle VM and Claude Desktop is on your laptop):

```json
{
  "mcpServers": {
    "brain": {
      "command": "npx",
      "args": [
        "-y",
        "mcp-remote",
        "http://<host-or-tailnet>:47778/mcp",
        "--allow-http"
      ]
    }
  }
}
```

`--allow-http` is required for any non-HTTPS endpoint. Prefer Tailscale,
Cloudflare Tunnel, or TLS in production (see §5, §6).

Restart Claude Desktop after editing the config (`Cmd+Q`, then relaunch).

### 4.2 Claude Code

Project-local config — create `.mcp.json` in the repo root:

```json
{
  "mcpServers": {
    "brain": {
      "command": "llm-wiki",
      "args": ["serve"]
    }
  }
}
```

User-global config lives at `~/.claude/config.toml` (or the equivalent
on your OS) and uses TOML:

```toml
[mcp_servers.brain]
command = "llm-wiki"
args = ["serve"]
```

There is also a plugin route: the `llm-wiki-skills` plugin starts
`llm-wiki serve` automatically and exposes 11 workflow skills as slash
commands. See `docs/guides/ide-integration.md`:

```bash
claude plugin add /path/to/llm-wiki-skills
```

### 4.3 Codex CLI

File: `~/.codex/config.toml`

```toml
[mcp_servers.brain]
command = "llm-wiki"
args = ["serve"]
```

Codex also accepts a project-local `.mcp.json` (same shape as Claude
Code):

```json
{
  "mcpServers": {
    "brain": {
      "command": "llm-wiki",
      "args": ["serve"]
    }
  }
}
```

Use an absolute binary path if the client cannot find `llm-wiki`.

### 4.4 Codex Desktop

Codex Desktop does not consume a remote `url` field directly. Bridge
HTTP MCP through `mcp-remote`:

```json
{
  "mcpServers": {
    "brain": {
      "command": "npx",
      "args": [
        "-y",
        "mcp-remote",
        "http://<host-or-tailnet>:47778/mcp",
        "--allow-http"
      ]
    }
  }
}
```

### 4.5 Cursor / Windsurf / Zed / VS Code

Full instructions live in `docs/guides/ide-integration.md`. Quick
shapes:

**VS Code** — `.vscode/mcp.json`:

```json
{
  "servers": {
    "llm-wiki": {
      "type": "stdio",
      "command": "llm-wiki",
      "args": ["serve"]
    }
  }
}
```

**Cursor** — `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "llm-wiki": {
      "command": "llm-wiki",
      "args": ["serve"]
    }
  }
}
```

**Windsurf** — Windsurf MCP config (same shape as Cursor):

```json
{
  "mcpServers": {
    "llm-wiki": {
      "command": "llm-wiki",
      "args": ["serve"]
    }
  }
}
```

**Zed (ACP)** — ACP streams workflow steps visibly in the agent panel.
When using `--acp`, you **must** also pass `--http` so MCP moves off
stdio (otherwise MCP and ACP compete for the same stdio stream):

```json
{
  "agent_servers": {
    "llm-wiki": {
      "type": "custom",
      "command": "llm-wiki",
      "args": ["serve", "--acp", "--http", ":47778"],
      "env": {}
    }
  }
}
```

### 4.6 Local stdio vs remote HTTP — trade-offs

| Aspect | Local stdio | Remote HTTP |
|---|---|---|
| Latency | Lowest (no network) | Network hop adds ~10 ms |
| Setup | One binary on laptop | Server + Tailscale/Tunnel |
| Multi-client | One process per client | One server, many clients |
| Zed ACP | Not viable (ACP needs stdio exclusively) | Required for ACP |
| Recovery surface | Local files only | Backup target on VM |
| Auth boundary | OS user | Tailscale/TLS + `http_allowed_hosts` |

Rule of thumb: run local stdio on your laptop when you only need one
client and the data lives locally. Switch to remote HTTP when (a) the
canonical wiki lives on the VM, (b) you want multiple clients to share
state, or (c) you use Zed ACP.

### 4.7 Verify the connection

Once a client is configured, ask the agent:

```
Search the wiki for "mixture of experts"
```

If the connection works, the agent calls `wiki_search` and returns
ranked results. If it fails, see §11 (MCP client cannot list tools).

---

## 5. HTTP MCP Setup

For remote access, run the server with HTTP transport. The endpoint
clients point at **must include `/mcp`**.

### 5.1 Start command

```bash
llm-wiki serve \
    --http :47778 \
    --watch \
    --web \
    --web-port 1414 \
    --web-bind 0.0.0.0
```

Flags:

- `--http :47778` — listen on TCP 47778 for MCP-over-HTTP.
- `--watch` — live-index filesystem changes (debounced, default 500 ms).
- `--web` — start the Hugo web UI alongside MCP.
- `--web-port 1414` — Hugo dev server port.
- `--web-bind 0.0.0.0` — bind the web UI on all interfaces. **Pair this
  with a reverse proxy or Tailscale only** (see §6). Do not expose
  `0.0.0.0:1414` to the public internet.

### 5.2 Allowed hosts

For remote clients the server checks the `Host` header. Allow the
hostnames clients will use:

```bash
llm-wiki config set --global \
    serve.http_allowed_hosts "localhost,127.0.0.1,::1,<public-ip-or-hostname>,<tailnet-name>"
```

### 5.3 Long-idle sessions

Streamable HTTP uses an MCP session ID after initialization. Some
remote bridges (notably `mcp-remote`) do not recover cleanly when the
server-side session expires while the client is idle. The default
keep-alive is 6 hours (21600 seconds):

```bash
llm-wiki config set serve.mcp_session_keep_alive_secs 21600 --global
```

Set to `0` **only** on private single-user servers where disabling idle
cleanup is acceptable:

```bash
llm-wiki config set serve.mcp_session_keep_alive_secs 0 --global
```

Related HTTP session knobs:

```bash
llm-wiki config set serve.mcp_init_timeout_secs 60 --global
llm-wiki config set serve.mcp_completed_cache_ttl_secs 60 --global
```

### 5.4 Endpoint URL

Clients must include `/mcp`:

```text
http://<host>:47778/mcp
```

Pointing a client at `http://<host>:47778` (without `/mcp`) returns the
root page or 404, never the MCP handshake.

### 5.5 TLS / HTTPS

The Brain server does **not** terminate TLS itself. Put one of these in
front:

- **Tailscale** — MagicDNS + HTTPS certs out of the box. Easiest path.
- **Cloudflare Tunnel** — no inbound ports on the VM; Cloudflare
  proxies outbound.
- **Caddy / Nginx reverse proxy** — terminate TLS, forward to
  `127.0.0.1:47778`.

Caddy minimal example (auto-HTTPS via Let's Encrypt):

```caddyfile
brain.example.com {
    reverse_proxy 127.0.0.1:47778
}
```

Nginx minimal example:

```nginx
server {
    listen 443 ssl http2;
    server_name brain.example.com;

    ssl_certificate     /etc/letsencrypt/live/brain.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/brain.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:47778;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto https;
        proxy_buffering off;   # streaming responses
    }
}
```

### 5.6 Strong warning

> Never expose the MCP server on a public port without authentication.
> The server stores private memory — operator profile, semantic claims,
> procedural runbooks, ingested sources. An unauthenticated `/mcp` on
> `0.0.0.0` is a data leak.

If you must expose HTTP, you **must** also have:

- TLS (Caddy/Nginx/Cloudflare) **and**
- Network ACL restricting source IPs **or**
- Tailscale-only bind (`--web-bind 100.x.x.x` if your tailnet IP is in
  the 100.64.0.0/10 CGNAT range) **or**
- A signed gateway token enforced by a reverse proxy.

Plain HTTP on `0.0.0.0:47778` is acceptable **only** for a few minutes
during first-boot debugging, on a host with no public ingress.

---

## 6. Network Security

The whole deployment philosophy is **no public MCP ports**. Two
recommended transports.

### 6.1 Tailscale

Install on the VM:

```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```

Authenticate in the browser window that opens. Note the tailnet name
assigned (`brain.tailnet.ts.net` or similar) — that is the hostname
clients will use.

MagicDNS gives you a stable hostname like `brain.tailnet.ts.net`. With
HTTPS enabled in the Tailscale admin panel, you get a valid TLS cert
automatically:

```bash
sudo tailscale cert brain.tailnet.ts.net
# Writes brain.tailnet.ts.net.crt and .key in the current directory.
```

Clients then point at:

```text
https://brain.tailnet.ts.net:47778/mcp
```

Or, for non-TLS internal tailnet traffic:

```text
http://brain.tailnet.ts.net:47778/mcp
```

ACL the tailnet so only your devices can reach `brain`. Tailscale ACLs
live in the admin console; the minimal policy denies everything by
default and allows your user's devices only.

### 6.2 Cloudflare Tunnel

No inbound ports on the VM; Cloudflare proxies outbound.

```bash
# Install cloudflared
curl -L https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64.deb -o cloudflared.deb
sudo dpkg -i cloudflared.deb

# Authenticate
cloudflared tunnel login

# Create a tunnel
cloudflared tunnel create brain
cloudflared tunnel route dns brain brain.example.com

# Run it (foreground for testing)
cloudflared tunnel --config ~/.cloudflared/config.yml run brain
```

Minimal `~/.cloudflared/config.yml`:

```yaml
tunnel: <tunnel-id>
credentials-file: /home/brain/.cloudflared/<tunnel-id>.json

ingress:
  - hostname: brain.example.com
    service: http://127.0.0.1:47778
  - service: http_status:404
```

Run as a systemd service for production:

```bash
sudo cloudflared service install
```

Cloudflare Tunnel gives you TLS, DDoS protection, and Cloudflare Access
policies for auth at the edge.

### 6.3 Firewall rules

On the VM itself, restrict inbound to the tailnet interface only:

```bash
# Allow Tailscale traffic
sudo ufw allow in on tailscale0
sudo ufw allow 22/tcp           # SSH — restrict source IP if possible

# Explicitly deny public MCP and web UI
sudo ufw deny 47778
sudo ufw deny 1414
sudo ufw deny 8765

# Default
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw enable
```

If you use Cloudflare Tunnel only, you do not need any inbound rules for
MCP — `cloudflared` keeps an outbound connection to Cloudflare.

### 6.4 Why

BLUEPRINT §9.3 is explicit: **MCP server stores private data — must not
be scannable.** Public exposure turns the knowledge base into an open
document store. Even read-only access leaks operator identity, project
decisions, and ingested source content. The cost of a Tailscale setup
(~10 minutes) is trivial compared to the cost of a leak.

---

## 7. Hugo Web UI

Every newly created wiki gets a Hugo scaffold in `site/`. The web UI is
a **read mirror**, not a source of truth.

### 7.1 Install / regenerate the scaffold

```bash
# On a brand-new wiki (auto-created, but explicit is safe)
llm-wiki web install --wiki brain

# Force regeneration after embedded web UI changes or an upgrade
llm-wiki web install --wiki brain --force
```

### 7.2 Dev preview

```bash
llm-wiki web serve --wiki brain
```

Serves Hugo's dev server at `http://127.0.0.1:1313/`. Requires
HugoExtended 0.147+ on `PATH`. `install.sh` tries to install Hugo
automatically; if it is missing:

```bash
# Linux ARM64
HUGO_VERSION=$(curl -fsSL https://api.github.com/repos/gohugoio/hugo/releases/latest \
    | grep '"tag_name"' | sed -E 's/.*"v([^"]+)".*/\1/')
curl -LO "https://github.com/gohugoio/hugo/releases/download/v${HUGO_VERSION}/hugo_extended_${HUGO_VERSION}_linux-arm64.tar.gz"
tar xzf "hugo_extended_${HUGO_VERSION}_linux-arm64.tar.gz"
sudo install -m 0755 hugo /usr/local/bin/
```

### 7.3 Production

Combine with serve flags so MCP, live index, and web UI share one
process:

```bash
llm-wiki serve --http :47778 --watch --web --web-port 1414 --web-bind 0.0.0.0
```

In production this is what `brain-mcp.service` runs (see §3).

### 7.4 Pages missing in the web UI

This is the most common web UI complaint. The fix is mechanical:

```bash
llm-wiki web install --wiki brain --force
sudo systemctl restart brain-mcp
```

**Do NOT edit `site/content/` directly.** `site/content/` is a generated
mirror that the engine rewrites on every `web install`. Hand edits will
be silently overwritten. The engine reads `wiki/` directly and regenerates
`site/content/` from the canonical Markdown.

The same rule applies when a section `index.md` does not show its child
pages: refresh the mirror and restart; do not patch the mirror.

---

## 8. Backup and Recovery

BLUEPRINT §9.4 sets the policy. Markdown + Git is the canonical layer;
everything else (Tantivy, Petgraph, vector index, generated wiki) is
rebuildable from it. In vnext the event ledger adds a second canonical
layer for claim state (see `references/architecture.md`).

### 8.1 Primary backup — daily git push

Schedule a daily `git push` of the brain wiki to a private GitHub or
Gitea repo at 00:00 UTC.

**Prepare the remote:**

```bash
# On the VM, as the brain user
cd ~/wikis/brain
git remote add origin git@github.com:<you>/brain-backup.git
ssh-keygen -t ed25519 -C "brain-backup" -f ~/.ssh/brain_backup -N ""
cat ~/.ssh/brain_backup.pub
# Add the public key as a deploy key (with write access) on GitHub.
```

Configure `~/.ssh/config` so the deploy key is used for the backup
remote:

```text
Host github-backup
    HostName github.com
    User git
    IdentityFile ~/.ssh/brain_backup
    IdentitiesOnly yes
```

Update the remote:

```bash
cd ~/wikis/brain
git remote set-url origin git@github-backup:<you>/brain-backup.git
```

**The script** — `/home/brain/bin/backup-brain.sh`:

```bash
#!/bin/bash
# Daily brain-mcp backup.
# Commits any uncommitted wiki changes, pushes to the private backup
# remote, and logs the result. Designed for cron.

set -euo pipefail

WIKI_DIR="${WIKI_DIR:-/home/brain/wikis/brain}"
LOG_FILE="${LOG_FILE:-/var/log/brain-backup.log}"
REMOTE="${REMOTE:-origin}"
BRANCH="${BRANCH:-main}"

log() {
    printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "$LOG_FILE"
}

cd "$WIKI_DIR"

# 1. Capture any local changes the engine committed but did not push.
if ! git diff --quiet || ! git diff --cached --quiet; then
    log "uncommitted changes detected; stashing for safety"
    git stash push -u -m "auto-stash before backup $(date -u +%FT%TZ)"
fi

# 2. Fetch + reconcile with the remote (fail-safe if remote was edited).
git fetch "$REMOTE" "$BRANCH" || {
    log "ERROR: fetch failed"
    exit 2
}

# 3. Push.
if git push "$REMOTE" "$BRANCH"; then
    log "OK: pushed $(git rev-parse --short HEAD) to $REMOTE/$BRANCH"
else
    log "ERROR: push failed"
    exit 3
fi

# 4. Sanity check: confirm remote head matches local head.
LOCAL=$(git rev-parse HEAD)
REMOTE_HEAD=$(git rev-parse "$REMOTE/$BRANCH")
if [ "$LOCAL" != "$REMOTE_HEAD" ]; then
    log "WARN: local $LOCAL != remote $REMOTE_HEAD after push"
fi
```

Install + permissions:

```bash
sudo install -d /var/log
sudo install -m 755 -o brain -g brain /home/brain/bin/backup-brain.sh /home/brain/bin/backup-brain.sh
sudo touch /var/log/brain-backup.log
sudo chown brain:brain /var/log/brain-backup.log
```

**The cron entry** — `/etc/cron.d/brain-backup`:

```cron
# Daily backup of the brain wiki to the private GitHub mirror.
# Runs at 00:00 UTC every day.
SHELL=/bin/bash
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

0 0 * * * brain /home/brain/bin/backup-brain.sh >> /var/log/brain-backup.log 2>&1
```

Install:

```bash
sudo install -m 644 /tmp/brain-backup.cron /etc/cron.d/brain-backup
sudo systemctl reload cron
```

Test manually before relying on cron:

```bash
sudo -u brain /home/brain/bin/backup-brain.sh
tail -n 20 /var/log/brain-backup.log
```

### 8.2 Qdrant snapshot (future)

When the vector tier is live (Phase 3+), add a weekly Qdrant snapshot
to Oracle Object Storage (Always Free includes 20 GB). Placeholder:

```bash
# Future: weekly Qdrant snapshot
# 0 3 * * 0 brain /home/brain/bin/snapshot-qdrant.sh
```

The snapshot script will: take a Qdrant snapshot, tar+gzip it, upload
via the OCI CLI, prune snapshots older than 30 days. The DR plan below
covers the case where the snapshot is missing — Qdrant rebuilds from
Markdown.

### 8.3 Disaster recovery

The git repo is the source of truth for the authored document layer.
With vnext, the event ledger is also canonical for claim state. Both
must be in the backup. The recovery rule (BLUEPRINT §9.4, §16):
**rebuild all derived indexes from Markdown + Git; replay the event
ledger for claim state.**

What is recoverable:

| Layer | Recovery |
|---|---|
| Markdown content | `git clone` the backup repo |
| Event ledger state (vnext) | Ledger replay through selected head |
| Tantivy index | `llm-wiki index rebuild` |
| Petgraph | Rebuilt from frontmatter on engine start |
| Vector index | Re-embed from Markdown (Phase 3+ DR drill target: <30 min at 1K pages) |
| Claim snapshot (vnext) | Rebuild from ledger head minus denied IDs at purge epoch |
| Generated wiki / Hugo site | `llm-wiki web install --force` |

What is **NOT** recoverable:

- **Purged bytes.** Per ADR-0001, hard purge destroys DEKs and rewrites
  Git history. After `registry_denied`, every read and decrypt fails
  closed. Purge is irreversible. There is no escrow.

### 8.4 Restore drill

Run this on a fresh VM (or any machine with `llm-wiki` installed) at
least once a quarter. It exercises the actual recovery path, not just
the backup.

```bash
# 1. Clone the backup repo to a fresh location.
git clone git@github-backup:<you>/brain-backup.git /tmp/brain-restore

# 2. Register it as a wiki space (no create, no overwrite).
llm-wiki spaces register /tmp/brain-restore --name brain-restore

# 3. Rebuild the Tantivy index from Markdown.
llm-wiki index rebuild --wiki brain-restore

# 4. Regenerate the Hugo mirror.
llm-wiki web install --wiki brain-restore --force

# 5. Spot-check.
llm-wiki search "operator" --wiki brain-restore | head
llm-wiki web serve --wiki brain-restore &
curl -sf http://127.0.0.1:1313/ | head

# 6. In vnext: verify the event ledger replayed by checking brain_status
#    via the MCP server (start it and call brain_status).
#    llm-wiki serve --http :47779
#    Then from an MCP client: call brain_status and confirm ledger_head,
#    claim_count, and schema_version match the production values.

# 7. Cleanup.
llm-wiki spaces remove brain-restore
rm -rf /tmp/brain-restore
```

If the drill fails, **fix the recovery path before you need it**. A
backup you cannot restore is not a backup.

### 8.5 Purge irreversibility — repeated for emphasis

Per ADR-0001 (see `references/architecture.md`): hard purge is a
seven-stage security operation. After stage 2 (`registry_denied`), every
read, decrypt, export, and restore fails closed — even if later cleanup
crashes. There is no recovery path that "undoes" a purge. The
irreversibility is the feature. Confirm with the user before running
any purge-style operation, and explain the blast radius.

---

## 9. Monitoring and Observability

### 9.1 Structured logs (JSONL)

Every MCP call is logged. Set JSON format for aggregator-friendly
parsing:

```bash
llm-wiki config set logging.log_format json --global
llm-wiki config set logging.log_max_files 30 --global
sudo systemctl restart brain-mcp
```

Log shape (from BLUEPRINT §12.1):

```json
{
  "ts": "2026-07-19T10:00:00Z",
  "client": "claude-desktop",
  "tool": "semantic_search",
  "args_hash": "sha256:...",
  "duration_ms": 423,
  "result_count": 10,
  "status": "ok"
}
```

Tailing via journald:

```bash
sudo journalctl -u brain-mcp -f -o cat
```

### 9.2 Prometheus metrics

The `/metrics` endpoint exposes standard Prometheus text exposition
format. Counters and gauges from BLUEPRINT §12.2 (and the Docker guide's
F2.2 metric reference):

| Metric | Kind | Labels |
|---|---|---|
| `brain_mcp_tool_calls_total` | counter | `tool`, `status` |
| `brain_mcp_tool_duration_seconds` | histogram | `tool` |
| `brain_pages_total` | gauge | `type`, `status` |
| `brain_index_freshness_seconds` | gauge | — |
| `brain_qdrant_query_duration_seconds` | histogram | — (future) |
| `brain_embedding_duration_seconds` | histogram | — (future) |
| `mcp_calls_total` | counter | `tool`, `status` |
| `projection_lag_seconds` | gauge | — |
| `job_queue_depth` | gauge | `state` |

Quick probe:

```bash
curl -sf http://127.0.0.1:47778/metrics | head -n 20
```

Sample Prometheus scrape config (sibling host or local agent):

```yaml
scrape_configs:
  - job_name: brain
    scrape_interval: 15s
    metrics_path: /metrics
    static_configs:
      - targets: ["127.0.0.1:47778"]
        labels:
          service: brain-mcp
```

Do **not** publish the metrics port on all interfaces without auth.

### 9.3 Health endpoint

```bash
# Liveness — always 200 while the process is up.
curl -sf http://127.0.0.1:47778/health

# Readiness — 200 only when DB + migrations + indexes healthy.
# (Available when the Console API is wired; check your build.)
curl -sf http://127.0.0.1:47778/ready
```

For load balancer routing decisions, point at `/ready`, not `/health`.
`/health` feeds the restart decision (process up?); `/ready` feeds the
routing decision (deps ok?).

### 9.4 Alerts (minimal baseline)

| Alert | Condition | Why |
|---|---|---|
| brain-mcp service down | `systemctl is-active brain-mcp != active` for 1 min | Service crashed and did not restart |
| Backup git push fail | non-zero exit from `backup-brain.sh` cron | Data is not leaving the host |
| Disk > 80% | `node_filesystem_avail_bytes` ratio | Index rebuilds and the event ledger need room |
| Latency p95 > hard limit | `brain_mcp_tool_duration_seconds` p95 over the hard limit for 5 min | Performance regression; see BLUEPRINT §11 |

Prometheus alert baseline (adapt to your metrics names):

```yaml
groups:
  - name: brain
    rules:
      - alert: BrainProjectionLagGrowing
        expr: projection_lag_seconds > 100
        for: 10m
        annotations:
          summary: "Projection lag above 100 events for 10m"

      - alert: BrainMcpErrorRateHigh
        expr: |
          sum(rate(mcp_calls_total{status="error"}[5m]))
          / sum(rate(mcp_calls_total[5m])) > 0.1
        for: 5m
        annotations:
          summary: "MCP tool error rate > 10%"
```

For service-down and backup-failed alerts, run a small shell loop on a
separate host (or use your existing monitoring) — these are not metrics
the server can publish about itself.

---

## 10. Update Workflow

### 10.1 Normal update

```bash
# 1. Pull the latest published GitHub Release.
curl -fsSL https://raw.githubusercontent.com/hataichanokpan-dev/brain-mcp/main/install.sh | bash

# 2. Verify version.
llm-wiki --version

# 3. Rebuild the Tantivy index (after schema or index changes).
llm-wiki index rebuild --wiki brain

# 4. Regenerate the Hugo mirror (after embedded web UI changes).
llm-wiki web install --wiki brain --force

# 5. Restart the service so the new binary picks up the new state.
sudo systemctl restart brain-mcp

# 6. Confirm.
sudo systemctl status brain-mcp --no-pager
sudo journalctl -u brain-mcp -n 50 --no-pager
```

### 10.2 The installer pulls a Release, not a tag

`install.sh` calls `GET /releases/latest` and downloads the matching
asset. If a release workflow is still building when you run it, the
script falls back to `install_from_source` (clones the repo and runs
`cargo build --release --locked`). That is fine but slow.

If you specifically want a Release asset that is mid-build, wait and
re-run. Do **not** assume the latest tag has downloadable assets.

### 10.3 When to run which step

- `index rebuild` — after any change that affects the Tantivy schema or
  indexing rules. Cheap insurance; safe to run any time search results
  look stale.
- `web install --force` — after any change to the embedded Hugo theme,
  templates, or scaffold. Also safe to run any time the web UI looks
  wrong.
- `systemctl restart brain-mcp` — after every binary update. The server
  cannot hot-reload the binary.

### 10.4 vnext: schema-version changes

In vnext, the event ledger has a `schema_version` field. If the
on-disk schema version advances (refuse-to-serve-until-upgraded policy),
the older store refuses to serve traffic until an operator explicitly
runs the upgrade subcommand:

```bash
# Plan only — leaves the store untouched.
llm-wiki recovery upgrade --dry-run

# Execute.
llm-wiki recovery upgrade

# Restart the server so its open() picks up the rewritten marker.
sudo systemctl restart brain-mcp
```

Before running the upgrade, verify the current schema version via
`brain_status` (via MCP) and read `references/architecture.md` for the
projection replay semantics. If `brain_status` reports a degraded
schema version, **do not write** — escalate before any mutation.

Rollback is a rehearsal tool, not a production path:

```bash
llm-wiki recovery upgrade --rollback
```

Running rollback in production after the server has accepted v3 traffic
leaves the store at v2 with a v3-era ledger, which then refuses to
serve until you re-run `recovery upgrade`. Rehearse rollback in staging
first.

---

## 11. Troubleshooting Playbook

Each subsection: symptom → cause → fix. Run commands in the order
shown.

### 11.1 MCP client cannot list tools

**Symptom:** the MCP client connects but `list_tools` returns empty, or
the client shows "no tools available."

**Cause:** one of — binary not on the client's `PATH`, stale index,
server startup error swallowed by the client.

**Fix:**

```bash
# 1. Use an absolute command path in the client's config.
#    Claude Desktop in particular spawns the server with a minimal env.
#    Replace "llm-wiki" with "/usr/local/bin/llm-wiki" (Linux)
#    or "/Users/<you>/.../llm-wiki" (macOS).

# 2. Restart the MCP client after every config edit.

# 3. Rebuild the index — the server may be refusing to serve because
#    the index is missing or stale.
llm-wiki index rebuild --wiki brain

# 4. Run serve manually to see startup errors the client swallows.
llm-wiki serve
# Look for: missing wiki, missing schema, schema version mismatch,
# port-in-use, file permission errors.
```

### 11.2 Remote clients get `ECONNREFUSED`

**Symptom:** remote client reports `ECONNREFUSED <host>:47778` (or
1414, 8765).

**Cause:** the server is not listening on the target port, or a
firewall is dropping the SYN.

**Fix:**

```bash
# 1. Is the service up?
sudo systemctl status brain-mcp --no-pager

# 2. What does journald say?
sudo journalctl -u brain-mcp -n 120 --no-pager

# 3. Is anything listening on the expected ports?
sudo ss -ltnp | grep -E ':47778|:1414|:8765'

# 4. If nothing is listening: start the service.
sudo systemctl start brain-mcp

# 5. If it is listening on 127.0.0.1 only, the bind flag is wrong.
#    The systemd ExecStart must include --web-bind 0.0.0.0 (or the
#    tailnet interface IP) for the web UI, and the HTTP MCP server
#    must bind 0.0.0.0 (or the tailnet IP) for remote clients.

# 6. Firewall.
sudo ufw status numbered
# Allow Tailscale inbound.
sudo ufw allow in on tailscale0
```

### 11.3 systemd shows `status=203/EXEC`

**Symptom:** `systemctl status brain-mcp` shows
`Failed with result 'exit-code'` and `status=203/EXEC`.

**Cause:** systemd could not exec `ExecStart`. The binary never started.

**Fix** (in order):

```bash
# 1. Does the binary exist at the configured path?
command -v llm-wiki
ls -l /usr/local/bin/llm-wiki
/usr/local/bin/llm-wiki --version

# 2. Permissions.
sudo chmod +x /usr/local/bin/llm-wiki

# 3. Architecture. Ampere = aarch64. An x86_64 binary exec-fails on ARM.
file /usr/local/bin/llm-wiki
# Expected: ELF 64-bit LSB executable, ARM aarch64

# 4. SELinux context (RHEL-derived; Ubuntu usually has none).
sudo restorecon -v /usr/local/bin/llm-wiki || true

# 5. Reload + restart.
sudo systemctl daemon-reload
sudo systemctl restart brain-mcp
sudo systemctl status brain-mcp --no-pager
```

If the binary is in `~/.cargo/bin` or `~/.local/bin`, either move it to
`/usr/local/bin` or change `ExecStart` to the absolute path. Relative
paths and `~` do not expand in unit files.

### 11.4 Web UI opens but pages are missing

**Symptom:** the Hugo site loads at `http://host:1413/` but page links
404, or child pages do not appear under a section.

**Cause:** the generated mirror under `site/content/` is stale or
missing entries.

**Fix:**

```bash
# 1. Refresh the mirror.
llm-wiki web install --wiki brain --force

# 2. Restart the service so the web UI re-reads the mirror.
sudo systemctl restart brain-mcp
```

**Do NOT edit `site/content/`.** The engine rewrites it on every
`web install`. Hand edits are silently overwritten. Edit `wiki/`
content (via MCP tools or `llm-wiki content write`) and refresh.

### 11.5 `brain_status` shows degraded schema version

**Symptom:** `brain_status` reports `schema_version` that does not match
the expected `CURRENT_DISK_SCHEMA_VERSION`, or reports the store as
`degraded` / `unhealthy`.

**Cause:** the binary was updated to a version that bumped the on-disk
schema; refuse-to-serve-until-upgraded is in effect; or the event
ledger head is inconsistent with the snapshot.

**Fix:**

```text
1. DO NOT WRITE. Stop all MCP clients that might mutate state.
2. Read references/architecture.md — the vnext authority model — to
   understand what schema_version and ledger_head mean here.
3. Escalate before any mutation. This is not a normal ops situation.
4. If an upgrade is genuinely intended:
     llm-wiki recovery upgrade --dry-run    # plan only
     llm-wiki recovery upgrade              # execute
     sudo systemctl restart brain-mcp
5. Re-check brain_status. If still degraded, do not retry with
   different payloads — read ADR-0001 first.
```

### 11.6 Git lock contention (`index.lock`)

**Symptom:** writes fail with `fatal: Unable to create
'<wiki>/.git/index.lock': File exists.`

**Cause:** another writer is mid-commit, or a previous writer crashed
and left the lockfile behind. The engine's retry logic usually handles
the first case; the second case is rare.

**Fix:**

```bash
# 1. Confirm no other writer is active.
sudo lsof +D ~/wikis/brain/.git 2>/dev/null
ps -ef | grep -E 'llm-wiki|brain' | grep -v grep

# 2. If nothing is running and the lock is stale, remove it.
ls -la ~/wikis/brain/.git/index.lock
sudo rm -f ~/wikis/brain/.git/index.lock

# 3. Verify the repo is healthy.
cd ~/wikis/brain
git status
git fsck --no-dangling

# 4. Restart the service so it picks up the clean state.
sudo systemctl restart brain-mcp
```

If locks recur, look for two writers — typically two `llm-wiki serve`
processes (one in tmux, one under systemd) pointing at the same wiki.
Kill one.

### 11.7 Idempotency conflict

**Symptom:** a write returns `IDEMPOTENCY_CONFLICT` with no mutation.

**Cause:** the client retried with the same `operation_id` but a
changed tool or payload. The 7-stage atomic write topology (see
`references/architecture.md`) detects this and refuses the second
attempt.

**Fix:**

```text
1. DO NOT retry with a changed payload. That defeats the idempotency
   contract.
2. Decide: is the original write the one you wanted?
     - If yes: the original already succeeded. Read it back
       (brain_get / semantic_get) and confirm.
     - If no: issue a NEW operation with a NEW operation_id that
       supersedes the original (brain_supersede for claims; for
       Markdown pages, write a new revision).
3. Read ADR-0001 for the full idempotency contract if the situation
   is unclear.
```

### 11.8 Quick reference: which log to read first

| Symptom | First log to read |
|---|---|
| MCP client cannot list tools | `llm-wiki serve` stderr (manual run) |
| Service will not start | `journalctl -u brain-mcp -n 200` |
| Remote ECONNREFUSED | `ss -ltnp` + `systemctl status` |
| Web UI missing pages | `llm-wiki web install --force` output |
| `brain_status` unhealthy | `references/architecture.md` |
| Backup did not run | `/var/log/brain-backup.log` |
| Slow search | `/metrics` histogram + `wiki_index_status` |

---

## 12. Hardening Checklist

Run this before declaring the deployment production-ready.

- [ ] **Tailscale or TLS configured.** No client reaches the server over
      plain HTTP on a public interface.
- [ ] **Public firewall blocks MCP ports.** `ufw status` shows `47778`,
      `1414`, `8765` denied for non-tailnet sources.
- [ ] **Daily backup cron scheduled.** `/etc/cron.d/brain-backup`
      exists; `backup-brain.sh` ran successfully at least once.
- [ ] **`brain_status` healthy.** Ledger head, claim count, and
      schema_version match expected values.
- [ ] **Hugo web UI password-protected or behind Tailscale.** If
      `--web-bind 0.0.0.0`, there is a reverse proxy with auth in
      front.
- [ ] **Disk < 70%.** Room for index rebuilds and the event ledger.
- [ ] **Logs rotating.** `journalctl --vacuum-time=30d` configured or
      `logging.log_max_files` set; `/var/log/brain-backup.log` under
      logrotate if it grows.
- [ ] **Schema version matches expected.** `brain_status` reports the
      current `CURRENT_DISK_SCHEMA_VERSION`; no `recovery upgrade`
      pending.
- [ ] **Bootstrap secret via file, not env** (if Console API is wired).
      See `docs/guides/deploy-docker.md` — never via `environment:`.
- [ ] **Restore drill passed.** §8.4 ran end-to-end on a fresh VM
      within the last quarter.

---

## 13. Further Reading

| Source | What it covers |
|---|---|
| `install.sh` (repo root) | Platform detection, binary install, Hugo bootstrap |
| `BLUEPRINT.md` §9 (repo root) | Deployment topology, host spec, networking, backup |
| `BLUEPRINT.md` §12 (repo root) | Monitoring: structured logs, metrics, alerts |
| `BLUEPRINT.md` §16 (repo root) | Risk register |
| `docs/guides/deploy-docker.md` | Docker / Compose deployment, `/health` and `/ready` semantics, encrypted backups, schema upgrades, RPO/RTO |
| `docs/guides/installation.md` | Binary install methods (script, cargo, manual download) |
| `docs/guides/configuration.md` | Config resolution, global vs per-wiki keys, common tuning |
| `docs/guides/ide-integration.md` | Per-client MCP config: VS Code, Cursor, Windsurf, Zed (ACP), HTTP transport, web preview |
| `docs/guides/multi-wiki.md` | Multiple wiki spaces, cross-wiki search, `wiki://` URIs |
| `SECURITY.md` (repo root) | Supported versions, vulnerability reporting, networked surfaces, dependency review |
| `references/architecture.md` (this skill) | Three stores, vnext authority model, recovery and audit, performance budget |
| `references/tool-reference.md` (this skill) | The 39-tool matrix with args, tiers, examples |
| `references/anti-patterns.md` (this skill) | Risky patterns to avoid before acting |
| `references/deployment-shapes.md` (this skill) | Shape A/B/C command reference; read before §0/§1 on an unfamiliar host |
| `references/workflows/operator.md` (this skill) | Day-to-day memory and knowledge tasks |
| `references/workflows/developer.md` (this skill) | Contributing to brain-mcp source |

When this playbook and the repo disagree, the repo wins.
