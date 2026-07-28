# Install Guide — Deploy brain-mcp on a VM (git clone → docker compose)

> คู่มือติดตั้งจากศูนย์บน VM (Linux, amd64 หรือ arm64) — clone จาก GitHub แล้วรันด้วย Docker Compose
> ฉบับละเอียด/เหตุผลเชิงลึกอยู่ที่ [`deploy-docker.md`](deploy-docker.md) — ไฟล์นี้คือ path ที่สั้นที่สุดจาก VM เปล่า → ระบบใช้งานได้
> Security model: **loopback-only** — server publish แค่ `127.0.0.1:8080` เข้าถึงจากเครื่องอื่นผ่าน SSH tunnel เท่านั้น

## 0. Prerequisites

- VM: Linux (Ubuntu/Debian แนะนำ), RAM ≥ 2GB, disk ≥ 10GB
- Docker Engine 24+ พร้อม Compose v2 (`docker compose version` ต้องผ่าน)
- `git`, `openssl`, `curl`
- Arch: ได้ทั้ง amd64 และ arm64 (Oracle Ampere) — เพราะ build image บน VM เอง (ไม่ pull binary)

```bash
# Ubuntu quick-start (ข้ามถ้ามี docker แล้ว)
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER   # logout/login หลังรัน
```

## 1. Clone

```bash
git clone <YOUR_GITHUB_REPO_URL> brain-mcp
cd brain-mcp
```

## 2. Operator layout (config + secret)

ทุกอย่างอยู่ที่ repo root — ทั้ง 4 dirs ถูก gitignore ไว้แล้ว (ห้าม commit):

```bash
mkdir -p config secrets data backups
(umask 077; openssl rand -hex 32 | tr -d '\r\n' > secrets/bootstrap_secret.txt)
cp examples/config.docker.toml config/config.toml
```

- `secrets/bootstrap_secret.txt` = รหัส login Console + auth ของ `/api/v1` — เก็บสำเนาไว้ใน password manager
- **ระวัง CRLF**: secret file ต้องเป็น 64 ตัวอักษร hex ไม่มี newline/`\r` (`wc -c` ต้องได้ 64) — ถ้ามี `\r` ติดมา login จะ 401 ตลอด

## 3. Build + start

```bash
docker compose up -d --build     # first build ~5–15 นาที
```

- Image build จาก `Dockerfile` multi-stage (Rust + Node console) → non-root (uid 1000), `restart: unless-stopped` = ขึ้นเองหลัง reboot
- ครั้งถัดๆ ไป (upgrade): `git pull && docker compose up -d --build`

## 4. Verify

```bash
curl -sf http://127.0.0.1:8080/health     # {"uptime_secs":...}
curl -sf http://127.0.0.1:8080/ready      # {"status":"ready","checks":{...all true}}

# auth gate: ผิด → 401, ถูก → 200 + csrf_token
curl -s -o /dev/null -w '%{http_code}\n' -X POST http://127.0.0.1:8080/api/v1/auth/login \
  -H 'Content-Type: application/json' -d '{"secret":"wrong"}'
curl -s -X POST http://127.0.0.1:8080/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d "{\"secret\":\"$(cat secrets/bootstrap_secret.txt)\"}"
```

ทั้งหมดผ่าน = ติดตั้งสำเร็จ · ถ้า `/ready` ตอบ 503 ดู `docker logs brain --tail 50`

## 5. Register the first wiki space (⚠ gotcha `:ro` config)

Compose mount config เป็น **read-only** (hardening) → `wiki_spaces_create` ผ่าน MCP จะ fail `Read-only file system (os error 30)` — ใช้วิธีนี้แทน:

```bash
# 5.1 scaffold ใน container ด้วย config ชั่วคราว (writable)
docker exec brain sh -c \
  'cp /data/config.toml /tmp/c.toml && LLM_WIKI_CONFIG=/tmp/c.toml \
   llm-wiki spaces create /data/wikis/brain --name brain --set-default'
# (Windows Git Bash: ใส่ MSYS_NO_PATHCONV=1 หน้า docker exec)

# 5.2 เพิ่ม registration ลง host config แล้ว restart
#     แก้ config/config.toml: ตั้ง default_wiki = "brain" ใน [global]
#     + เพิ่ม array-of-tables entry ของ wiki (name = "brain",
#     path = "/data/wikis/brain") — TOML syntax เต็มดูใน
#     skills/brain/references/deployment-shapes.md
docker restart brain

# 5.3 verify
curl -sf http://127.0.0.1:8080/ready
```

Wiki data ต้องอยู่ใต้ `/data` (volume) เท่านั้น — ที่อื่นหายตอน recreate container

## 6. Connect MCP clients

```bash
# Claude Code บน VM เอง:
claude mcp add -s user -t http brain http://127.0.0.1:8080/mcp

# จากเครื่อง laptop/desktop: เปิด SSH tunnel ก่อน แล้ว add URL เดียวกัน
ssh -N -L 8080:127.0.0.1:8080 user@your-vm &
claude mcp add -s user -t http brain http://127.0.0.1:8080/mcp
```

ทดสอบ: เปิด Claude Code session ใหม่ → `/mcp` ต้องเห็น server `brain` (39 tools) → สั่ง "call brain_status" ต้องได้ `status: healthy`

Console UI: ผ่าน tunnel เดียวกัน → เปิด `http://127.0.0.1:8080/` login ด้วย bootstrap secret

## 7. Operations ประจำ

```bash
# encrypted backup (AES-256-GCM) — output ต้องอยู่ใต้ /data
docker exec brain llm-wiki --config /data/config.toml recovery backup \
  --output /data/backup-$(date +%Y%m%d) --format json

# restore drill (พิสูจน์ว่า backup restore ได้จริง — รันหลัง backup ทุกครั้ง)
docker exec brain llm-wiki --config /data/config.toml recovery drill \
  --backup-dir /data/backup-$(date +%Y%m%d) \
  --key-file /data/semantic-store/backup.key --format json

# metrics / logs / สถานะ
curl -s http://127.0.0.1:8080/metrics | head
docker logs brain --tail 100
```

**⚠ Backup key** อยู่ที่ `/data/semantic-store/backup.key` — **copy ออกนอกเครื่องเก็บแยกทันทีหลังติดตั้ง** (`docker cp brain:/data/semantic-store/backup.key ...`) ไม่งั้น disk พัง = backup ถอดรหัสไม่ได้ = ข้อมูลหายถาวร

## 8. กฎ security (ห้ามละเมิด)

1. **Public exposure requires Bearer auth on /mcp.** Set `BRAIN_MCP_TOKENS`
   (comma-separated, multi-token) and `http_bind_all_interfaces=true` in
   config. Without tokens, public bind is fail-closed (server refuses to
   start). See `docs/plans/2026-07-28-oracle-vm-migration-bearer-auth-design.md`
   for the full deployment runbook (Oracle Cloud, ARM64, no-TLS).
   Loopback-only deployments (the original quickstart) need neither.
2. ต้องการเข้าถึงจากภายนอก → SSH tunnel เท่านั้น (หรือ reverse proxy + TLS + auth ค่อยพิจารณาเป็น project แยก)
3. `secrets/`, `config/config.toml`, `data/`, `backups/` ถูก gitignore แล้ว — อย่า force-add
4. Rotate secret: เขียนไฟล์ใหม่ (ขั้นตอน §2) → `docker compose up -d` (recreate เพื่ออ่าน secret ใหม่)

## 9. Troubleshooting

| อาการ | สาเหตุ/ทางแก้ |
|---|---|
| `/ready` 503 | store/index ยังไม่พร้อม — ดู `docker logs brain`; ถ้า schema mismatch ดู recovery ใน deploy-docker.md |
| login 401 ทั้งที่ secret ถูก | `\r`/newline ติดใน secret file — regen ตาม §2 แล้ว recreate container |
| `wiki_spaces_create` → os error 30 | by design (`:ro` config) — ใช้ §5 |
| MCP client ต่อไม่ติด | tunnel หลุด / session เก่าเปิดก่อน `claude mcp add` — เปิด session ใหม่ |
| container ไม่ขึ้นหลัง reboot | `docker compose up -d` หนึ่งครั้ง (restart policy ผูกกับ docker daemon) |
| `docker compose up` แล้วได้ code เก่า | image tag ค้าง — `docker compose up -d --build` เสมอเวลา upgrade |
