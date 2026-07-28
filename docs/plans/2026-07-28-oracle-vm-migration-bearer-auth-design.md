---
title: "Migrate brain-mcp-vnext to Oracle Cloud VM (ARM64) with multi-token Bearer auth on /mcp"
date: 2026-07-28
status: approved
author: brainstorming session with operator (THP)
tags: [deployment, security, oracle-cloud, arm64, bearer-auth]
---

# Design: Oracle Cloud VM Migration + Bearer Auth on /mcp

## Context

`brain-mcp-vnext` ปัจจุบันรันบน Docker Desktop ของ Windows ที่ `127.0.0.1:8080` (loopback-only) — trust boundary SEC-1 ตาม decision `loopback-pre-gate-golive` (2026-07-19): `/mcp` ไม่มี per-caller auth เพราะยอมรับว่าเข้าถึงได้แค่ loopback

เป้าหมาย: ย้ายไป **Oracle Cloud Free Tier Ampere A1 (ARM64)** แบบ **public HTTP** (operator ยอมรับความเสี่ยงไม่มี TLS) พร้อมเพิ่ม **multi-token Bearer auth บน `/mcp`** เพื่อเปิด public ได้อย่างปลอดภัย

Production use — เก็บข้อมูลจริง, ต้อง backup, ต้อง security review

## Operator decisions (locked from brainstorming)

| # | Decision | Choice | Rationale |
|---|---|---|---|
| 1 | VM target | Oracle Cloud Free Tier (Ampere A1 ARM64) | Existing VM พร้อม + Docker ลงแล้ว |
| 2 | Build method | Build native ARM64 บน VM (`docker compose up --build`) | ไม่ต้อง cross-arch, smoke test ARM ไปในตัว |
| 3 | Data migration | เอาข้อมูลเดิม (data/, backups/, config/, rules/, secrets/) ไปด้วย | รักษา knowledge ที่สะสมไว้ |
| 4 | Network exposure | Public HTTP (no TLS) | Operator accept risk; TLS เป็น phase ถัดไป |
| 5 | `/mcp` auth | Bearer multi-token | ทุก client ได้ token ของตัวเอง |
| 6 | Token storage | Env var `BRAIN_MCP_TOKENS` (comma-separated) | สอดคล้อง pattern `BRAIN_USERNAME`/`BRAIN_PASSWORD` |
| 7 | Auth approach | Custom axum middleware (Approach B) | Multi-token natural, ควบคุม logging/error เอง |

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ Internet (browser, MCP clients: Claude/Cursor/scripts)      │
│                          │                                   │
│                          ▼ HTTP plain text (operator accept) │
│ ┌─────────────────────────────────────────────────────────┐ │
│ │ Oracle Cloud VM (Ampere A1 ARM64, Oracle Linux)         │ │
│ │  ┌──────────────────────────────────────────────────┐   │ │
│ │  │ Docker Engine                                     │   │ │
│ │  │  ┌────────────────────────────────────────────┐  │   │ │
│ │  │  │ brain container (linux/arm64, native build)│  │   │ │
│ │  │  │  :8080                                       │  │   │ │
│ │  │  │  ├─ /health, /ready    [no auth]            │  │   │ │
│ │  │  │  ├─ /                  [console login]      │  │   │ │
│ │  │  │  ├─ /api/v1/*         [session+CSRF]        │  │   │ │
│ │  │  │  └─ /mcp              [Bearer multi-token] ← NEW│  ││ │
│ │  │  │                                              │  │   │ │
│ │  │  │  Volumes (migrated from Windows host):       │  │   │ │
│ │  │  │   /data         ← data/ (~9 MB)             │  │   │ │
│ │  │  │   /backups      ← backups/ (~4 MB)          │  │   │ │
│ │  │  │   /data/config.toml ← config/ (ro)          │  │   │ │
│ │  │  │   /data/rules    ← rules/ (ro)              │  │   │ │
│ │  │  │  Env (from .env on VM):                      │  │   │ │
│ │  │  │   BRAIN_USERNAME, BRAIN_PASSWORD (console)   │  │   │ │
│ │  │  │   BRAIN_MCP_TOKENS (comma-sep) ← NEW         │  │   │ │
│ │  │  │   ZAI_API_KEY (provider)                     │  │   │ │
│ │  │  └────────────────────────────────────────────┘  │   │ │
│ │  └──────────────────────────────────────────────────┘   │ │
│ │                                                          │ │
│ │  Oracle Security List: open :8080 from 0.0.0.0/0        │ │
│ └─────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

**Auth layers (separation of concerns):**
- `/health`, `/ready`, `/metrics`: no auth (liveness probes)
- `/` (console SPA + login form): public load, login ด้วย username/password
- `/api/v1/*`: session cookie + CSRF token (existing, unchanged)
- `/mcp`: **Bearer multi-token** (new) — gates all MCP tool calls

## Bearer Auth Design (the net-new code)

### Files touched

| File | Change | Approx LOC |
|---|---|---|
| `src/config.rs` | เพิ่ม field `mcp_bearer_tokens_env` + `resolve_mcp_tokens()` method | +30 |
| `src/server.rs` | Wire middleware เข้า router (both stateful/stateless branches) | +15 |
| `src/mcp/bearer_auth.rs` (new) | Custom axum middleware: extract header, constant-time compare against list | ~50 |
| `src/mcp/mod.rs` | `pub mod bearer_auth;` | +1 |
| `src/api.rs` | promote `constant_time_eq` จาก private → `pub(crate)` เพื่อ reuse | +0 (เปลี่ยน visibility) |
| `.env.example` | เพิ่ม `BRAIN_MCP_TOKENS=` (commented, with gen instructions) | +5 |
| `docker-compose.yml` | ส่ง `BRAIN_MCP_TOKENS=${BRAIN_MCP_TOKENS:-}` เข้า container | +1 |

### Config field + resolver (mirrors `resolve_bootstrap_credentials`)

```rust
// src/config.rs — ใน ServeConfig struct
/// Comma-separated Bearer tokens accepted on /mcp. Stored as env-var NAME
/// (not value), same pattern as BRAIN_PASSWORD. Empty + public bind → fail-closed.
#[serde(default = "default_mcp_bearer_tokens_env")]
pub mcp_bearer_tokens_env: String,  // default "BRAIN_MCP_TOKENS"

fn default_mcp_bearer_tokens_env() -> String { "BRAIN_MCP_TOKENS".into() }

pub fn resolve_mcp_tokens(&self) -> Result<Vec<String>, ServeConfigError> {
    let raw = std::env::var(&self.mcp_bearer_tokens_env).unwrap_or_default();
    let tokens: Vec<String> = raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    // Fail-closed: public exposure + no tokens = refuse to start
    if self.http_bind_all_interfaces && tokens.is_empty() {
        return Err(ServeConfigError::PublicExposureWithoutMcpAuth);
    }
    Ok(tokens)
}
```

### Custom middleware (src/mcp/bearer_auth.rs)

```rust
//! Bearer token auth middleware for /mcp.
//!
//! Multi-token: each MCP client gets its own token (BRAIN_MCP_TOKENS,
//! comma-separated). Constant-time compare against the list to avoid
//! timing-attack token enumeration. Logs only "missing"/"mismatch" —
//! never the token value (TokenRedaction already covers log scrubbing).

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
// Reuse constant_time_eq from src/api.rs (already audited, no new dep needed).
use crate::api::constant_time_eq;

pub fn layer(tokens: Arc<Vec<String>>) -> axum::middleware::FromFnLayer<
    fn(State<Arc<Vec<String>>>, HeaderMap, Next) -> _,
    _,
> {
    axum::middleware::from_fn_with_state(tokens, require_bearer)
}

async fn require_bearer(
    State(tokens): State<Arc<Vec<String>>>,
    headers: HeaderMap,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(provided) = extract_bearer(&headers) else {
        tracing::warn!("mcp auth: missing or malformed Authorization header");
        return Err(StatusCode::UNAUTHORIZED);
    };
    let matched = tokens
        .iter()
        .any(|t| constant_time_eq(t.as_bytes(), provided.as_bytes()));
    if !matched {
        tracing::warn!("mcp auth: token mismatch");
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(headers).await)
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ").map(str::trim)
}
```

### Wiring in server.rs

```rust
// หลังสร้าง mcp_service (stateful branch ~line 232, stateless ~line 245)
let mcp_tokens = Arc::new(serve_cfg.resolve_mcp_tokens()?);
let mcp_router = if mcp_tokens.is_empty() {
    // Loopback-only mode (backward compatible with dev setup)
    axum::Router::new().nest_service("/mcp", mcp_service)
} else {
    axum::Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(crate::mcp::bearer_auth::layer(mcp_tokens))
};
```

### Key design decisions

| ประเด็น | การตัดสินใจ | เหตุผล |
|---|---|---|
| Token format | Comma-separated ใน `BRAIN_MCP_TOKENS` env var | สอดคล้อง `BRAIN_USERNAME`/`BRAIN_PASSWORD` |
| Token comparison | `subtle::ConstantTimeEq` | กัน timing attack (เหมือน console login) |
| Missing header | 401 + log warn | ตาม RFC 6750 |
| Token mismatch | 401 + log warn (ไม่บอก "wrong token") | กัน enumeration |
| Loopback fallback | ถ้า tokens ว่าง + `http_bind_all_interfaces=false` → ไม่ติด middleware | Backward compatible |
| Fail-closed | `bind_all_interfaces=true` + tokens ว่าง → refuse to start | ห้ามเปิด public โดยไม่มี auth |
| Logging | log แค่ "missing"/"mismatch" ไม่ log token | กัน leak token ผ่าน log |
| Rate limiting | ไม่ใส่ใน scope นี้ | YAGNI — ทีหลังผ่าน reverse proxy |

### Test plan

1. **Unit** (`src/mcp/bearer_auth.rs`): missing header, malformed header (`Basic xxx`, `Bearer` ไม่มี value), valid token, invalid token, multi-token match (ทุกตัวใน list), multi-token no-match
2. **Integration**: spin up server ด้วย `BRAIN_MCP_TOKENS=token1,token2`, ยิง `/mcp`:
   - ไม่มี Authorization → 401
   - `Authorization: Bearer token1` → pass through to mcp handler
   - `Authorization: Bearer wrong` → 401
3. **Fail-closed**: `http_bind_all_interfaces=true` + `BRAIN_MCP_TOKENS` ว่าง → server panic/error ตอน boot
4. **Backward-compat**: `http_bind_all_interfaces=false` (loopback) + ไม่มี `BRAIN_MCP_TOKENS` → server start ปกติ, `/mcp` ไม่ต้อง auth

## Deployment Runbook (9 steps)

### Phase A — บนเครื่อง Windows (เตรียม + push)

**Step 1 — Push source ไป remote** ที่จะใช้ clone บน VM
- ถ้ามี git remote: `git push origin <branch>` (หลัง commit Bearer auth code)
- ถ้าไม่มี: `rsync -av --exclude target --exclude node_modules --exclude .git ./ ubuntu@<VM>:~/brain-mcp-vnext/`
- **Verify:** `ls brain-mcp-vnext/` บน VM มี `Cargo.toml`, `Dockerfile`, `web/console/`

**Step 2 — Migrate data** (Windows → VM)
```bash
# ฝั่ง Windows: สร้าง tarball
tar -czf brain-data.tar.gz data/ backups/ config/ rules/ secrets/

# scp ไป VM
scp brain-data.tar.gz ubuntu@<VM_PUBLIC_IP>:~/

# ฝั่ง VM: แตกไว้ที่ repo root
ssh ubuntu@<VM_PUBLIC_IP>
mkdir -p ~/brain-mcp-vnext && cd ~/brain-mcp-vnext
tar -xzf ~/brain-data.tar.gz
```
- **Verify:** `du -sh data/ backups/` ตรงกับฝั่ง Windows (~13 MB รวม)
- **⚠️ Critical:** `data/semantic-store/backup.key` คือ AES-256 key — copy เก็บ password manager ด้วย (off-host)

### Phase B — บน VM (build + config)

**Step 3 — ติดตั้ง prerequisites** (skip ถ้ามีแล้ว)
```bash
curl -fsSL https://get.docker.com | sudo sh
sudo usermod -aG docker $USER
# logout + login ใหม่
```
- **Verify:** `docker --version && docker compose version`

**Step 4 — สร้าง `.env`** บน VM
```bash
cd ~/brain-mcp-vnext
cp .env.example .env
# generate 3 tokens (one per client):
for i in 1 2 3; do openssl rand -hex 32; done
# copy ผลลัพธ์ 3 บรรทัด ไปใส่ใน .env:
#   BRAIN_MCP_TOKENS=<token1>,<token2>,<token3>
#   BRAIN_USERNAME=<console user>
#   BRAIN_PASSWORD=<strong password>
#   ZAI_API_KEY=<ถ้ามี>
```
- **Verify:** `cat .env` (เช็คครบ, ห้าม commit `.env` เข้า git!)

**Step 5 — Build native ARM64 image** (ครั้งแรก 10-15 นาที)
```bash
docker compose up -d --build
```
- **Verify:** `docker compose ps` (status: Up, healthy)
- **Verify:** `docker compose logs brain | head -50` (ดู error หรือ "listening on :8080")
- **⚠️ If ARM build fails:** Dockerfile ไม่มี arch-specific commands แต่ deps อาจมี quirks; budget time + read error logs

**Step 6 — เปิด Oracle Security List** ให้ :8080 เข้าจาก internet
```
Oracle Cloud Console → Networking → VCN → Security Lists → Ingress Rules
Add: Source 0.0.0.0/0, IP Protocol TCP, Dest Port 8080
```
- **Verify:** จากเครื่องอื่น `curl http://<VM_PUBLIC_IP>:8080/health` → ต้องได้ `{"status":"ok"}` หรือ similar
- **Note:** อาจต้อง configure iptables ใน VM ด้วย (`sudo iptables -I INPUT -p tcp --dport 8080 -j ACCEPT` + persist)

### Phase C — Verify + harden

**Step 7 — Verify Bearer auth ทำงาน**
```bash
# ไม่มี token → 401
curl -i http://<VM_PUBLIC_IP>:8080/mcp
# Expected: HTTP/1.1 401 Unauthorized

# Token ถูก → pass (อาจเป็น 405 หรือ 200 แล้วแต่ method)
curl -i -H "Authorization: Bearer <token1>" http://<VM_PUBLIC_IP>:8080/mcp

# Token ผิด → 401
curl -i -H "Authorization: Bearer wrong-token" http://<VM_PUBLIC_IP>:8080/mcp
```
- **Verify:** log `docker compose logs brain | grep "mcp auth"` เห็น warn messages

**Step 8 — Verify console login**
```
เปิด browser: http://<VM_PUBLIC_IP>:8080/
Login ด้วย BRAIN_USERNAME / BRAIN_PASSWORD
ต้องเห็นหน้า Today/Home เหมือนเดิม + data ครบ
```

**Step 9 — เก็บ backup.key off-host** (critical — ทำทันที)
```bash
# Copy ออกจาก VM ไป password manager / secure storage
scp ubuntu@<VM_PUBLIC_IP>:~/brain-mcp-vnext/data/semantic-store/backup.key ~/secure-backup.key
```

## Known risks (operator-accepted)

| Risk | Impact | Mitigation |
|---|---|---|
| **ไม่มี TLS** | Bearer token + login cookie ส่ง plain text ผ่าน internet — ใครดักจับ (ISP, MITM) อ่าน token และ session ได้ | Operator accept; phase ถัดไปเพิ่ม Caddy auto-TLS (Let's Encrypt) |
| **Port 8080 เปิด public** | Internet scanner ยิง `/mcp`, `/api/v1/*` ตลอด | Bearer auth + fail-closed; Oracle security list จำกัด IP ได้ทีหลัง |
| **ARM build ไม่เคย smoke test** | Deps อาจมี ARM-specific quirks | Budget time ตอน Step 5; Dockerfile พอร์ตได้ตาม Explore report (statically linked, multi-arch base images) |
| **Single point of failure** | VM down = service down | Oracle Free Tier SLA; `restart: unless-stopped` ใน compose; backup + restore drill สำคัญ |
| **Token รั่ว** | ถ้า token รั่วทุก client ที่ใช้ token นั้นเสีย | Multiple tokens (แยกต่อ client) → revoke แค่ client ที่รั่ว; rotation = generate ใหม่ + update env + restart |

## Non-goals (out of scope)

- **TLS termination** — phase ถัดไป (Caddy + Let's Encrypt)
- **Rate limiting / DDoS protection** — ทีหลังผ่าน reverse proxy
- **OAuth/SSO** — YAGNI, Bearer token พอสำหรับ use case นี้
- **Web Application Firewall** — ทีหลัง
- **Multi-VM / HA / clustering** — single VM พอสำหรับ Free Tier
- **Automated backup rotation** — manual copy ไป off-host ตามรอบที่ operator กำหนด

## Success criteria

1. ✅ brain-mcp-vnext รันบน Oracle VM ARM64 + Docker (native build, ไม่ใช้ QEMU)
2. ✅ Data เดิม (~13 MB) อยู่ครบ และ console แสดงผลเหมือนเดิม
3. ✅ `/mcp` ตอบ 401 ถ้าไม่มี Bearer token หรือ token ผิด
4. ✅ `/mcp` ตอบ 200/pass ถ้ามี Bearer token ที่อยู่ใน `BRAIN_MCP_TOKENS`
5. ✅ Console login (`/`) ใช้ได้ปกติด้วย username/password (เหมือนเดิม)
6. ✅ Server refuse to start ถ้า `http_bind_all_interfaces=true` + `BRAIN_MCP_TOKENS` ว่าง
7. ✅ Server backward-compatible: ถ้า `http_bind_all_interfaces=false` (loopback) + ไม่มี tokens → server start ปกติไม่มี auth

## Open questions (none — all resolved in brainstorming)

## References

- Decision: `decisions/loopback-pre-gate-golive` (current trust boundary)
- Existing doc: `docs/guides/install-vm.md` (loopback-only VM quickstart — §8 rules จะถูก update)
- Existing doc: `docs/guides/deploy-docker.md` (general Docker deploy — ARM section จะถูก update)
- Code: `src/server.rs:231-262` (current /mcp mounting)
- Code: `src/config.rs:460-489` (resolve_bootstrap_credentials — pattern ที่จะ mirror)
- Code: `src/api.rs:473-482` (constant_time_eq — จะ reuse)
