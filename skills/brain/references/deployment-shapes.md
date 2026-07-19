---
title: "Deployment Shapes"
summary: "The three ways brain-mcp runs in practice — local dev, Docker compose, systemd bare-metal — with per-shape start/restart/upgrade/backup commands and a detection heuristic. Read this before assuming any command in the other references works verbatim."
read_when:
  - Operating brain-mcp on a machine you have not confirmed the shape of yet
  - A command from another reference file (systemctl, docker exec, llm-wiki serve) errors unexpectedly
  - Setting up a new instance (laptop, VM, container host)
  - wiki_spaces_create or any write tool fails with a filesystem error
audience: any agent or human operating brain-mcp on any host
last_updated: "2026-07-19"
---

# Deployment Shapes

This skill is written to be **deployment-agnostic** — the same `SKILL.md`
and references apply whether brain-mcp runs on a laptop, a Docker host, or
a bare-metal VM. The one thing that differs across hosts is *how the
process is started, restarted, and how its config is writable*. This file
is the single place that varies; every other reference file should link
here instead of assuming a shape.

**Rule:** never hardcode a shape's specifics (path, port, restart command)
into `SKILL.md` or the other reference files. If you learn a new fact
about *this* instance specifically, it belongs in that instance's own
wiki (`entities/brain-instance.md`), not in the skill.

---

## The three shapes

| | Shape A: Local dev | Shape B: Docker compose | Shape C: systemd bare-metal |
|---|---|---|---|
| Process | `llm-wiki serve` run directly (stdio or `--http`) | container `brain`, image `brain:vX.Y` | `llm-wiki` binary under systemd unit `brain-mcp.service` |
| Config | any path, always writable | bind-mounted, **often `:ro`** by design (hardening) | plain file, writable by the service user |
| Typical host | contributor's laptop | any Docker host: a workstation, a VM, a cloud instance | Oracle Cloud Always Free ARM VM (BLUEPRINT §9 default target) |
| Network default | loopback / stdio | loopback published (`127.0.0.1:PORT:PORT`); widen only behind TLS+auth | bound to a private interface (Tailscale) or loopback behind reverse proxy |
| Persistence | wherever `--config` points | named volume / bind mount (`./data`, `./backups`) | plain filesystem paths under the service user's home or `/var/lib` |

BLUEPRINT §9 names Shape C (Oracle ARM + systemd) as the reference
production target. In practice, any of the three is a legitimate
production shape — pick by what the operator already has. **Docker
compose (Shape B) is the shape actually running as of 2026-07-19** on the
instance this skill was authored against; do not assume Shape C is live
anywhere until the instance descriptor says so (see below).

---

## Detecting the shape you're on

Before trusting any restart/upgrade command, determine the shape:

```bash
# 1. Is there a wiki page that already says?
wiki_content_read(uri: "entities/brain-instance")   # preferred — see below

# 2. No descriptor page yet — infer:
docker ps --filter name=brain          # container running? -> Shape B
systemctl status brain-mcp 2>&1        # unit exists? -> Shape C
ps aux | grep "llm-wiki serve"         # bare process, no systemd? -> Shape A
```

If you determine the shape and no `entities/brain-instance` page exists,
propose creating one (see § Instance descriptor below) — this turns a
one-time investigation into a fact every future session can read in one
call instead of re-detecting.

---

## Shape A: Local dev

```bash
llm-wiki spaces create ~/wikis/brain --name brain --set-default
llm-wiki serve                                      # stdio MCP
llm-wiki serve --http :47778                        # HTTP MCP (endpoint: /mcp)
llm-wiki serve --http :47778 --watch --web --web-port 1414 --web-bind 0.0.0.0
llm-wiki index rebuild --wiki brain
llm-wiki web install --wiki brain --force
```

Config is whatever file `--config` (or `LLM_WIKI_CONFIG`) points at — no
read-only constraints. All MCP write tools work as documented in
`tool-reference.md` with no shape-specific gotchas.

Restart: kill and re-run the `serve` command (no process manager).

---

## Shape B: Docker compose

Reference compose file: `docker-compose.yml` in the `brain-mcp-vnext`
repo. Key properties that differ from Shape A:

- **Config is bind-mounted read-only**: `./config/config.toml:/data/config.toml:ro`.
  This is deliberate hardening (the operator's config should not be
  writable by a process reachable over MCP), not an oversight.
- **Data/backups are writable volumes**: `./data:/data`, `./backups:/backups`.
- **The bootstrap secret is a Docker secret file**, never baked into the
  image or placed in `environment:` (env values leak via `docker inspect`).
- **Publish is loopback-only** (`127.0.0.1:PORT:PORT`) by default. A
  reverse proxy with TLS must sit in front for any non-loopback exposure
  — this applies identically whether the Docker host is a laptop or a
  cloud VM. See anti-pattern #10 in `anti-patterns.md`.

### Start / restart / logs

```bash
docker compose up -d --no-build          # start (image already built/tagged)
docker compose up -d --build             # rebuild image, then start
docker restart brain                     # restart after a config change
docker compose logs -f brain             # follow logs
docker compose down                      # stop (volumes persist)
```

### Upgrade

```bash
docker build -t brain:vX.Y+1 .
# edit docker-compose.yml: image: brain:vX.Y+1  (or use --build)
docker compose up -d
curl -sf http://127.0.0.1:PORT/ready     # verify readiness gate
```

### Backup / restore (MCP-driven, not shape-specific)

Backup/restore/drill are exposed as MCP-adjacent HTTP ops
(`/ops/backup-health`, `recovery backup`, `recovery drill` — see the repo's
`docs/guides/deploy-docker.md`), not through the `brain_*` MCP tools. They
work the same regardless of shape; only the volume paths differ (`./backups`
in compose vs. an operator-chosen path in Shape A/C).

### ⚠️ `wiki_spaces_create` / any config-writing MCP tool fails with EROFS

**Symptom:** `wiki_spaces_create` (or any tool that persists a registry
entry to the config file) returns `Read-only file system (os error 30)`.

**Cause:** the tool writes the new space registration into the server's
config file. Compose mounts that file `:ro` on purpose. The wiki scaffold
itself *does* get created (it lands on the writable `/data` volume) — only
the config-file write fails.

**Fix (keeps the `:ro` hardening — do not remount config as `:rw`):**

```bash
# 1. Scaffold inside the container with a throwaway config copy
docker exec brain sh -c \
  'cp /data/config.toml /tmp/c.toml && \
   LLM_WIKI_CONFIG=/tmp/c.toml llm-wiki spaces create /data/wikis/<name> --name <name> --set-default'

# 2. Copy the registration into the HOST config file
cat >> ./config/config.toml <<'EOF'

[global]
default_wiki = "<name>"

[[wikis]]
name = "<name>"
path = "/data/wikis/<name>"
EOF

# 3. Restart to pick up the new registration
docker restart brain
curl -sf http://127.0.0.1:PORT/ready
```

On Windows hosts running Git Bash, prefix `docker exec` with
`MSYS_NO_PATHCONV=1` — otherwise Git Bash mangles absolute container
paths like `/data/...` into a Windows path.

Wiki data must live under the mounted data volume (`/data` in the
reference compose file) or it will not survive container recreation.

### Windows-host notes (compose on a Windows Docker Desktop host)

- `MSYS_NO_PATHCONV=1` before any `docker exec` invoked from Git Bash.
- `openssl rand -hex 32 | tr -d '\n'` still leaves a trailing `\r` on
  Windows when writing the secret file — strip `'\r\n'`, not just `'\n'`.
- See `references/workflows/developer.md` for the full Windows Rust build
  quirks (unrelated to Docker, but common on the same host).

---

## Shape C: systemd bare-metal

Full walkthrough (VM provisioning, systemd unit, TLS/reverse proxy,
Tailscale) lives in `references/workflows/deployer.md`. Summary:

```bash
# systemd unit at /etc/systemd/system/brain-mcp.service
sudo systemctl daemon-reload
sudo systemctl enable --now brain-mcp
sudo systemctl restart brain-mcp
sudo systemctl status brain-mcp --no-pager
journalctl -u brain-mcp -n 120 --no-pager
```

Config is a plain file the service user owns — no `:ro` constraint, no
EROFS gotcha. `wiki_spaces_create` and all other MCP write tools work
exactly as documented in `tool-reference.md`.

**This shape is BLUEPRINT design intent (§9), not independently verified
by this skill's authors against a running instance** — treat commands
here as review-only until confirmed against a live Shape C host, then
update this note.

---

## Instance descriptor: `entities/brain-instance`

To avoid re-detecting the shape every session, propose (per the
propose-then-confirm write discipline in `SKILL.md`) a page in the
instance's own wiki:

```yaml
---
title: "This brain-mcp instance"
type: entity
status: active
tags: [meta, deployment]
---

- shape: B (Docker compose)
- host: <laptop|vm-name>
- endpoint: http://127.0.0.1:<port>/mcp
- config_writable: false (:ro mount — see deployment-shapes.md § EROFS)
- restart: `docker restart <container-name>`
- image: <image:tag currently deployed>
```

This page is instance-specific fact, not skill content — it belongs in
`wiki/entities/`, never in this skills directory. Read it as the first
bootstrap step (see `SKILL.md` § Session Bootstrap) before falling back
to the detection heuristic above.

---

## Reference Index

| Reference | When to read |
|---|---|
| `references/architecture.md` | Why: three stores, event-ledger authority — shape-independent |
| `references/tool-reference.md` | How: the 39-tool matrix — shape-independent except the EROFS gotcha (linked from here) |
| `references/workflows/deployer.md` | Deep Shape C walkthrough (VM provisioning, systemd, TLS) |
| `references/workflows/operator.md` | Day-to-day tasks — shape-independent |
| `references/workflows/developer.md` | Contributing to brain-mcp source — shape-independent |

When this file and a specific instance's `entities/brain-instance` page
disagree, the instance page wins (it describes reality; this file
describes the space of possibilities).
