# Deploy brain-mcp ลง Oracle Cloud VM (สั้น กระชับ)

> คู่มือติดตั้งจากศูนย์ → ใช้งานได้ บน Oracle Cloud Free Tier (Ampere A1, ARM64)
> ครอบคลุม: สร้าง VM + เปิด firewall, ติดตั้ง + Bearer auth, เชื่อม MCP client
> ฉบับลึก/เหตุผลดู [`deploy-docker.md`](deploy-docker.md) และ [`install-vm.md`](install-vm.md) (generic VM, loopback-only)

## 0. สิ่งที่ต้องมี

- บัญชี Oracle Cloud (Free Tier พอ — Always Free quota: 4 OCPU / 24 GB RAM บน Ampere A1)
- SSH key pair สำหรับ login VM
- GitHub repo URL ของ brain-mcp ตัวเอง

---

## 1. สร้าง VM + เปิด firewall

### 1.1 สร้าง instance ใน Oracle Console

`Compute → Instances → Create instance` ตั้งค่าดังนี้:

| ฟิลด์ | ค่า |
|---|---|
| Name | `brain-mcp` (ตามใจ) |
| Image | **Canonical Ubuntu 22.04** (หรือ 24.04) — เป็น ARM64 image |
| Shape | **VM.Standard.A1.Flex** — ตั้ง 4 OCPU / 24 GB (ภายใน Always Free) |
| SSH keys | เพิ่ม public key ของคุณ |
| VCN | default (สร้างใหม่ถ้ายังไม่มี) |

หลักสร้างเสร็จ → บันทึก **public IP** ของ instance

### 1.2 เปิด port 8080 ใน VCN Security List

`Networking → Virtual Cloud Networks → <vcn ของคุณ> → Security Lists → Default Security List → Add Ingress Rules`:

| ฟิลด์ | ค่า |
|---|---|
| Source Type | CIDR |
| Source CIDR | `0.0.0.0/0` |
| IP Protocol | TCP |
| Destination Port Range | `8080` |

### 1.3 เปิด iptables ใน VM (Oracle Ubuntu image default block port)

```bash
ssh ubuntu@<PUBLIC_IP>
sudo iptables -I INPUT 6 -m state --state NEW -p tcp --dport 8080 -j ACCEPT
sudo netfilter-persistent save
```

---

## 2. ติดตั้ง + ตั้ง Bearer auth

### 2.1 ติดตั้ง Docker + clone repo

```bash
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER   # logout/login หลังรัน
newgrp docker                   # หรือ logout/login แล้ว ssh กลับเข้ามาใหม่

git clone <YOUR_REPO_URL> brain-mcp && cd brain-mcp
```

### 2.2 เตรียม secret + token

```bash
mkdir -p config secrets data backups

# bootstrap secret = login Console UI (/) + auth ของ /api/v1
(umask 077; openssl rand -hex 32 | tr -d '\r\n' > secrets/bootstrap_secret.txt)

# สร้าง MCP Bearer token (1 ตัวต่อ client — แยกด้วยจุลภาค)
echo "BRAIN_MCP_TOKENS=$(openssl rand -hex 32)" > .env
```

> 💡 วางแผน token: ควรมีอย่างน้อย 1 ตัวต่อ client (เช่น `tok-claude,tok-cursor,tok-scripts`) เวลาหลุดสามารถ rotate เฉพาะ client นั้นได้โดยไม่ต้องเปลี่ยนหมด

### 2.3 ตั้งค่า `config/config.toml` — เปิด public bind

```bash
cp examples/config.docker.toml config/config.toml
```

ตรวจ/แก้ใน `[serve]` ให้เป็นดังนี้:

```toml
[serve]
http = true
http_port = 8080
http_bind_all_interfaces = true   # 0.0.0.0 — เปิด public, บังคับใช้ Bearer auth
```

> ⚠️ **สำคัญ (fail-closed feature):** ถ้าตั้ง `http_bind_all_interfaces=true` แต่ไม่มี `BRAIN_MCP_TOKENS` ใน `.env` server **จะไม่ยอม start** เพื่อกันเปิด `/mcp` โดยไม่มี auth โดยไม่ตั้งใจ

### 2.4 Build + start

```bash
docker compose up -d --build    # first build ประมาณ 5–15 นาทีบน ARM
```

### 2.5 ตรวจว่าใช้งานได้

```bash
# health/ready ต้อง 200 (ไม่ต้องใช้ token — by design สำหรับ load balancer/probe)
curl http://127.0.0.1:8080/health
curl http://127.0.0.1:8080/ready     # {"status":"ready",...}

# /mcp ต้อง 401 เมื่อไม่มี token (auth ทำงานปกติ)
curl -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/mcp

# /mcp ต้อง 200 เมื่อใส่ token ถูก
TOKEN=$(grep BRAIN_MCP_TOKENS .env | cut -d= -f2)
curl -H "Authorization: Bearer $TOKEN" \
     -o /dev/null -w '%{http_code}\n' \
     http://127.0.0.1:8080/mcp
```

| ผลลัพธ์ | ความหมาย |
|---|---|
| `/health` 200 + `/ready` 200 | server พร้อม |
| `/mcp` ไม่มี token → 401 | auth ทำงานปกติ ✅ |
| `/mcp` มี token → 200 | token ถูก ✅ |
| `/mcp` ตอบ 200 ทั้งสองกรณี | มีอะไรผิด — ดู `docker logs brain --tail 50` |

---

## 3. เชื่อม MCP client

เอา `<PUBLIC_IP>` + `<token>` จากขั้นตอนที่แล้ว ไปใส่ใน client config

### Claude Desktop (`claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "brain": {
      "url": "http://<PUBLIC_IP>:8080/mcp",
      "headers": {
        "Authorization": "Bearer <token>"
      }
    }
  }
}
```

### Cursor / Claude Code CLI

```bash
claude mcp add -s user -t http \
  --header "Authorization: Bearer <token>" \
  brain http://<PUBLIC_IP>:8080/mcp
```

### ทดสอบ

เปิด session ใหม่ → `/mcp` ต้องเห็น server `brain` (ราว 39 tools) → ลองสั่ง `brain_status` ต้องได้ `status: healthy`

---

## หมายเหตุ security

สิ่งที่รูปแบบนี้ **ยังไม่ครอบคลุม** (ไม่ได้ทำในคู่มือนี้):

- **ไม่มี HTTPS** — token วิ่งบน HTTP plaintext ระหว่าง client กับ VM ใคร sniff ระหว่างทางเห็นหมด
  - ถ้าใช้ผ่าน internet จริงๆ → ควรเพิ่ม Caddy/nginx + Let's Encrypt ครอบไว้
  - หรือใช้ SSH tunnel แทน public bind (`ssh -N -L 8080:127.0.0.1:8080 user@vm`)
- **Console UI** (`/`) ใช้ cookie auth ของตัวเอง — ไม่ใช้ Bearer token ตัว Bearer ทำงานเฉพาะ `/mcp`
- `/health` และ `/ready` **ไม่ต้องใช้ token** (by design — ให้ probe เรียกได้)

## Operations ประจำ (สั้น)

```bash
# ดู log
docker logs brain --tail 100 -f

# upgrade (pull โค้ดใหม่ + rebuild)
git pull && docker compose up -d --build

# rotate token (เปลี่ยน .env แล้ว recreate)
# แก้ BRAIN_MCP_TOKENS ใน .env → แจ้ว clients ทุกตัวให้ใช้ token ใหม่ →
docker compose up -d   # recreate เพื่ออ่าน .env ใหม่

# backup (encrypted) — output ต้องอยู่ใต้ /data
docker exec brain llm-wiki --config /data/config.toml recovery backup \
  --output /data/backup-$(date +%Y%m%d) --format json
```

⚠️ **Backup key** อยู่ที่ `/data/semantic-store/backup.key` — copy ออกนอกเครื่องเก็บทันที (`docker cp brain:/data/semantic-store/backup.key ...`) ถ้า disk พัง + ไม่มี key = ข้อมูลถอดรหัสไม่ได้ = หายถาวร
