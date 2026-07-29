# Profile & Rules — วิธีสร้างกฎ/ตัวตน/สไตล์ให้ brain

> Profile = "constitution" ของ operator — กฎ ตัวตน สไตล์ ที่ brain โหลดทุก session ผ่าน `profile_get`
> ใช้บอก agent ว่าคุณเป็นใคร ทำงานยังไง ใช้อะไร และห้ามทำอะไร
> อ้างอิง: `BLUEPRINT.md` §2.1, `schemas/profile.json`, `docs/bug-reports/2026-05-23-profile-get-empty-pages.md`

## ภาพรวม

Profile คือหน้า markdown พิเศษที่มี `type: profile` ใน frontmatter แบ่งเป็น **5 sections**:

| Section | เก็บอะไร | ตัวอย่าง |
|---|---|---|
| `rules` | กฎแข็ง/กฎอ่อน ที่ห้าม/ควรทำ | "ห้าม emoji ใน commit", "ก่อน commit ต้องรัน test" |
| `identity` | ตัวตน บทบาท บริบท | "ชื่อ THP, Senior SWE @ Microchip, WFH Bangkok" |
| `style` | สไตล์การตอบ/เขียน | "ตอบไทยคละ EN, ไม่ต้องสรุปย่อ" |
| `stack` | เครื่องมือ/เทคโนโลยีหลัก | "TypeScript + Bun + ESM, Rust สำหรับ side project" |
| `constraints` | ข้อจำกัดแวดล้อม | "Windows + WSL2, เวลาทำงานเอเชีย" |

**ทุก section โหลดเข้า working memory ตอน session เริ่ม** → ขนาดต้องเล็ก (1–3 KB รวม) เพื่อโหลดได้ใน <100ms

---

## 2. Frontmatter schema

อ้างอิง `schemas/profile.json` — field ที่บังคับและตัวเลือก:

```yaml
---
title: "Hard Rules"                    # required — ชื่อ display
type: profile                          # required — const "profile"
section: rules                         # required — rules | identity | style | stack | constraints
priority: hard                         # required — hard | soft (hard = ห้ามฝ่าฝืน)
status: active                         # required — active | superseded
supersedes: profile/rules/old-rules    # optional — slug ของหน้าที่มาแทน
created: 2026-07-29                    # optional — ISO date
last_verified: 2026-07-29              # optional — ISO date ที่ตรวจสอบล่าสุด
tags: [commit-policy, language]        # optional — สำหรับค้นหา
---
```

**ค่าที่ใช้ได้ (enum จาก schema):**
- `section`: `rules` | `identity` | `style` | `stack` | `constraints` (เลือกได้ค่าเดียวต่อหน้า)
- `priority`: `hard` (ห้ามฝ่าฝืน เช่น "ห้าม force push") | `soft` (prefer เช่น "ชอบใช้ functional style")
- `status`: `active` (ใช้งาน) | `superseded` (เลิกใช้แล้ว — มีหน้าใหม่มาแทน)

---

## 3. Layout ที่ควรวาง

ตาม `BLUEPRINT.md` §2.1:

```
profile/
  identity.md              ← section: identity
  hard-rules.md            ← section: rules, priority: hard
  soft-preferences.md      ← section: rules, priority: soft
  style-guide.md           ← section: style
  stack.md                 ← section: stack
  constraints.md           ← section: constraints
```

> 💡 ไม่บังคับตามชื่อไฟล์ — `profile_get` กรองด้วย `section` field ใน frontmatter ไม่ใช่ชื่อไฟล์ แต่วางตามนี้เพื่ออ่านง่าย

---

## 4. ตัวอย่างเต็ม

### `profile/identity.md`

```markdown
---
title: "Operator Identity"
type: profile
section: identity
priority: hard
status: active
created: 2026-07-29
last_verified: 2026-07-29
tags: [operator, identity]
---

# ตัวตน

- **ชื่อ:** Thitipat (THP)
- **บทบาท:** Senior Software Engineer @ Microchip Technology
- **ที่อยู่:** Bangkok, WFH
- **ภาษา:** ไทย + EN (technical term ใช้ EN ไม่ต้องแปล)
- **Background:** 10+ ปี .NET/C# + TypeScript, gRPC, clean architecture, Docker, CI/CD
```

### `profile/hard-rules.md`

```markdown
---
title: "Hard Rules"
type: profile
section: rules
priority: hard
status: active
created: 2026-07-29
last_verified: 2026-07-29
tags: [commit-policy, security, workflow]
---

# กฎแข็ง (ห้ามฝ่าฝืน)

- **ห้าม commit / push โดยไม่ได้รับอนุญาต** — ต้องมีคำสั่ง "commit" / "ship" ชัดเจน
- **ก่อน commit ต้องรัน lint + test + typecheck ให้ผ่าน**
- **ห้าม `--force`, `--no-verify`, `git push -f` โดยไม่ได้รับอนุญาต**
- **ห้ามแตะ `.env`, secrets, credentials โดยไม่ได้ขอ explicit**
- **ห้าม log secrets/tokens ลง brain หรือ log file**
```

### `profile/style-guide.md`

```markdown
---
title: "Style Preferences"
type: profile
section: style
priority: soft
status: active
created: 2026-07-29
last_verified: 2026-07-29
tags: [communication, code-style]
---

# สไตล์การทำงาน

- **ภาษาตอบ:** ไทย — technical term เป็น EN ได้ ("race condition", "idempotent")
- **น้ำเสียง:** ตรง กระชับ engineer-to-engineer ไม่เป็นทางการ
- **ความยาว:** สั้นเป็น default — ลึกเฉพาะตอนซับซ้อนจริง
- **ห้ามขึ้นต้นด้วย** "Great question!", "I'd be happy to help", คำขอบคุณก่อนตอบ
- **ห้าม bullet list** ในเรื่องที่ตอบเป็น 1–2 ประโยคได้
```

### `profile/stack.md`

```markdown
---
title: "Tech Stack"
type: profile
section: stack
priority: soft
status: active
created: 2026-07-29
last_verified: 2026-07-29
tags: [stack, tools]
---

# Stack หลัก

- **งานหลัก:** .NET / C# + TypeScript
- **Side project:** Rust (brain-mcp-vnext)
- **CLI tools:** rg, fd, bat, jq, gh
- **Editor:** VS Code
- **Container:** Docker Desktop + WSL2 backend
```

### `profile/constraints.md`

```markdown
---
title: "Environment Constraints"
type: profile
section: constraints
priority: hard
status: active
created: 2026-07-29
last_verified: 2026-07-29
tags: [environment, os]
---

# ข้อจำกัดแวดล้อม

- **OS:** Windows 11 + WSL2 (Ubuntu)
- **Shell:** zsh (WSL) / PowerShell 7 (native)
- **Path separator:** ระวัง `\` vs `/` ในสคริปต์ cross-platform
- **Line ending:** git ตั้ง `core.autocrlf=input` เพื่อกัน CRLF ปนใน secret file
```

---

## 5. วิธีเขียนเข้า brain

Profile ใช้ **write path เดียวกับ page ทั่วไป** — ผ่าน `wiki_content_write` แล้ว `wiki_ingest`:

### วิธีที่ 1: ผ่าน MCP tool (แนะนำ)

```
brain_invoke wiki_content_write
  uri = "profile/hard-rules"
  content = <markdown พร้อม frontmatter ข้างบน>
  status = "active"

# หลัง write ต้อง ingest เพื่อ index
brain_invoke wiki_ingest path = "profile/hard-rules.md"
```

### วิธีที่ 2: เขียนไฟล์ตรง + ingest

วางไฟล์ `profile/<name>.md` ใน wiki root แล้วสั่ง:

```
brain_invoke wiki_ingest path = "profile/"
```

### วิธีที่ 3: CLI

```bash
llm-wiki content write --uri profile/hard-rules --file hard-rules.md
llm-wiki ingest --path profile/
```

---

## 6. Lifecycle — supersession (versioning)

Profile **ไม่มี decay** (ไม่เก่า/ลืมเอง) แต่มี **supersession** — หน้าใหม่มาแทนของเก่า:

1. สร้างหน้าใหม่ `status: active` + `supersedes: <slug ของของเก่า>`
2. เปลี่ยนหน้าเก่าเป็น `status: superseded`
3. `profile_get` จะ return เฉพาะ `status: active` เท่านั้น

ตัวอย่าง — ปรับกฎ commit:

```markdown
# หน้าใหม่: profile/hard-rules-v2
---
title: "Hard Rules v2"
type: profile
section: rules
priority: hard
status: active
supersedes: profile/hard-rules   ← บอกว่ามาแทนของเก่า
created: 2026-08-15
---

# หน้าเก่า: profile/hard-rules — เปลี่ยน status เป็น superseded
---
status: superseded
---
```

> 💡 การเปลี่ยน profile **ควรผ่าน `profile_propose_update` + confirm diff** (ตาม `BLUEPRINT.md` §2.1) เพื่อให้มี audit trail ใน ledger

---

## 7. ทดสอบว่าใช้งานได้

หลัง write + ingest แล้ว เรียก `profile_get` เพื่อตรวจ:

```
brain_invoke profile_get                       ← ดูทุก section
brain_invoke profile_get section = "rules"     ← กรองเฉพาะ rules
```

**ผลลัพธ์ที่ถูก:** `pages` array มีข้อมูล, ไม่ว่าง

### ⚠️ ถ้า `pages: []` ทั้งที่เขียนแล้ว

นี่คืออาการของ bug ที่เคยเกิด (`docs/bug-reports/2026-05-23-profile-get-empty-pages.md`) — root cause คือ `type` field ใน index ถูก tokenize/stem จน query exact match ไม่ติด

**แก้:** rebuild index

```bash
llm-wiki index rebuild --wiki <wiki-name>
# หรือใน docker:
docker exec brain llm-wiki --config /data/config.toml index rebuild --wiki brain
```

ตรวจอีกครั้ง — ต้องไม่ว่างแล้ว

---

## 8. กฎการเขียน profile ที่ดี

| หลักการ | ทำ | ห้าม |
|---|---|---|
| **ขนาด** | 1–3 KB รวมทุก section | ยาวเป็น essay — โหลดช้า + context overflow |
| **เนื้อหา** | กระชับ actionable (อ่านแล้วรู้ทันทีว่าทำ/ไม่ทำอะไร) | อธิบายเหตุผลยาว — เอาไปไว้ใน `concepts/` แทน |
| **Priority** | `hard` เฉพาะของจริงที่ห้ามฝ่าฝืน | ทำทุกอย่าง `hard` — สูญเสียนัยสำคัญ |
| **Scope** | global operator preference | project-scoped fact (เอาไป `projects/`) |
| **Freshness** | update `last_verified` ทุกครั้งที่ทบทวน | ปล่อยให้ค้างปี — ไม่รู้ว่ายัง valid ไหม |
| **Versioning** | supersede ของเก่า อย่าลบ | ลบไฟล์ — เสีย audit trail |

### สิ่งที่ **ไม่** ควรเป็น profile

- ❌ Project fact เฉพาะ ("brain-mcp ใช้ Rust 1.85") → `projects/brain-mcp-vnext`
- ❌ Concept ความรู้ ("Reciprocal Rank Fusion คือ...") → `concepts/`
- ❌ Workflow ที่ execute ได้ ("deploy สเต็ปต่างๆ") → `procedural/`
- ❌ Decision ("เลือก loopback-only go-live") → `decisions/`

Profile เก็บเฉพาะ **ตัวตน/กฎ/สไตล์/stack/constraints ระดับ operator** เท่านั้น
