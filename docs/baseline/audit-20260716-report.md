# Verification Audit Report — Phase A (Tasks A1 + A2 + A3)

- Date: 2026-07-16
- Base: `vnext/phase-0` @ `549c090` (audit ran after hygiene commit `3a21daf`)
- Plan: `docs/robustness-plan.md`
- Auditor: Claude (Builder+Orchestrator role per GOAL-vNext decision 26)

---

## 1. Task A1 — Repo hygiene + toolchain (CLOSED)

| Item | Finding | Disposition (user-approved) |
|---|---|---|
| `tests/fixtures/wikis/alt-root/schemas/{procedure,profile,semantic}.json` untracked ×3 | Known test side effect since Task 0.3: Python integration suite writes generated default schemas into tracked fixture | Committed at `3a21daf`; root-cause fix (copy fixture to temp before register) → backlog |
| `.zcode/` untracked | Session-tool plan artifact (Task 2.2 plan; content already in decision history) | Added to `.gitignore` at `3a21daf` |
| Toolchain versions | rustc/cargo 1.95.0, uv 0.11.7, cargo-audit 0.22.2 | Match baseline exactly |
| **Toolchain host triple** | rustup default host = **GNU**; `rust-toolchain.toml` pins only `channel="1.95"` → resolved to `1.95-x86_64-pc-windows-gnu` → **build broken** (`dlltool.exe not found` in getrandom) | Fixed with `rustup override set 1.95-x86_64-pc-windows-msvc` (repo-local, reversible). Classification: **environment**. Version-string check alone cannot catch this (GNU/MSVC print identically) |
| `CARGO_TARGET_DIR` | Env redirects all builds to `C:\Users\bfipa\.cargo-target`; `./target/debug/llm-wiki.exe` was a stale Jul 13 copy | Stale copy deleted; Python suites must use `LLM_WIKI_BIN` → CARGO_TARGET_DIR binary. Classification: **environment footgun**, documented |
| `Cargo.lock` sha256 vs `repository-environment.json` | Differs from captured baseline hash | Expected drift (Task 1.2b added RustCrypto deps); working copy == HEAD (EOL-only diff). Classification: **stale-doc** |
| `tests-integration/uv.lock` | Still gitignored → integration env not reproducible from repo (risk carried since Task 0.1) | Confirmed still open → backlog |

## 2. Task A2 — Gates re-run: reported vs actual (CLOSED)

All commands run 2026-07-16 on Windows 11, MSVC toolchain, `-j 2` where noted.

| # | Gate | Reported (last evidence packets) | Actual (this audit) | Verdict |
|---|---|---|---|---|
| 1 | `cargo fmt --check` | clean | exit 0 | ✅ match |
| 2 | `cargo clippy --all-targets -- -D warnings` | clean | exit 0 (after MSVC override; **fails on GNU resolution** — see A1) | ✅ match (env fix required) |
| 3 | `cargo clippy --all-targets --all-features -- -D warnings` | clean | exit 0 | ✅ match |
| 4 | `cargo test -j 2` (default) | ~700+ pass / 0 fail | **~803 pass / 0 fail**, exit 0 | ✅ match (growth from Phase 5–7 suites) |
| 5 | `cargo test --all-features -j 2` | 0 fail | **~810 pass / 0 fail**, exit 0 | ✅ match |
| 6 | eval v1 locked runner | "126/126 cases pass"; byte_lock false carried | cases 126/126 ✅, thresholds ✅, environment ✅, **byte_lock FAIL → overall exit 1** | ⚠️ match but note: **the eval gate as a whole FAILS**; reports quoting "126/126" were accurate on cases while the gate exit code is 1 |
| 7 | Python engine/mcp/acp | 63 / 76 / 26+2skip | 63 pass / 76 pass / 26 pass+2 skip | ✅ exact match |
| 8 | governance | 3 fail pre-existing | **3 fail / 7 pass** — all 3 root-caused to byte-lock mismatch (same 9 files) | ✅ exact match |
| 9 | `cargo audit` | "4/4" (1 vuln + 3 warnings) | **1 vuln: crossbeam-epoch 0.9.18 RUSTSEC-2026-0204** (invalid pointer deref in `fmt::Pointer`; fix ≥0.9.20 available). Warnings: bincode 2.0.1 (unmaintained), anyhow 1.0.102 (unsound `downcast_mut`), memmap2 0.9.10 (unsound ptr offset) | ✅ exact match; CVE identified, upgrade path exists |

Eval metrics actual: recall@10 = 1.0, nDCG@10 = 0.973, evidence-span exactness = 1.0, hard-invariant pass rate = 1.0, confirmed unsupported claims = 0.

Byte-lock mismatched files (9): `docs/adr/0001-…md`, `docs/security/threat-model-v1.md`, `evals/v1/contracts/event-schema-v1.json`, `evals/v1/metrics.json`, `evals/v1/run.py`, `evals/v1/cases/{stocks,projects,knowledge,adversarial}.jsonl` — i.e. the locked contract files were edited after the Task 0.2 hash-lock without re-locking the manifest.

**Conclusion:** ตัวเลขใน evidence packets ตรงกับความจริงทั้งหมด ไม่มี fabrication; discrepancy ที่พบเป็น environment 2 รายการ (GNU toolchain, CARGO_TARGET_DIR) + stale-doc 1 + carried debt ที่รายงานไว้แล้ว (byte-lock, audit)

## 3. Task A3 — §11 System DoD gap matrix

Legend: ✅ done (runtime-proven) · 🟨 contract-only (types+tests exist, not wired) · ❌ missing

| §11 bullet | สถานะ | Evidence |
|---|---|---|
| Claude Code/Codex/Inspector ผ่าน contract suite เดียวกัน stdio/HTTP | 🟨 | Task 3.2 contract tests; no real-client run |
| Confirmed claims 100% valid provenance union | ✅ (core) | semantic.rs gates + eval invariants; real-corpus proof via MCP pending |
| No silent overwrite; replay → same checksum | ✅ | Task 0.3/1.x/2.1 runtime tests, re-run green |
| Retry/concurrent writes no duplicate/lost update | ✅ | Task 1.3 suites, re-run green |
| GULF temporal scenario current + as-of | 🟨 | eval cases pass; end-to-end via MCP client not possible yet (no `brain_*` tools) |
| Latest-user-wins scoped; contradictions coexist | ✅ | Task 1.2/1.3 suites |
| Console ค้น/เพิ่ม/แก้/approve/purge/provenance/timeline/graph | 🟨→❌ | contract types only (console.rs/galaxy.rs/trust.rs); no React app |
| XSS/prompt-injection/auth-bypass/SSRF/path-traversal suites | 🟨 | unit/structural only; no runtime suite vs real transport/console |
| Encrypted backup + clean-host restore drill | 🟨 | recovery.rs types; `backup_consistent` exists (Task 1.1); real drill deferred |
| Index ลบและ rebuild จาก canonical | ✅ | Task 2.1 projection rebuild, re-run green |
| Domain eval ผ่าน threshold, no critical regression | ⚠️ | metrics pass but **byte-lock broken → gate exit 1** — eval integrity must be restored before use as promotion gate |
| Production TLS/least-privilege/audit/health/upgrade/rollback | 🟨→❌ | deployment.rs contracts; no real deployment |
| No trade-execution tools; investment output shows uncertainty | ✅ | no such tool exists (structural) |

## 4. Security risk report (severity-ranked)

| # | Severity | Finding | Evidence | Exposure today |
|---|---|---|---|---|
| S1 | **HIGH** | MCP Streamable HTTP binds `0.0.0.0` (all interfaces) with **no authentication**; only protection = Host-header allowlist (spoofable) | `src/server.rs:85` `([0,0,0,0], port)`; violates GOAL §7.1 loopback-only-until-auth | Latent: only when `serve --http` is run; any LAN/port-forward reachability = full unauthenticated read/write |
| S2 | **HIGH** (carried) | `AuthPolicy::allows` not wired into `call_tool` dispatch — capability model not enforced at runtime; per-handle capability gap from Phase 1 | `AuthPolicy` referenced nowhere outside `src/mcp/auth.rs` (grep-verified) | Blocks any transport exposure; must close before Phase C interop |
| S3 | MEDIUM | Eval byte-lock broken (9 locked files edited post-lock) → locked eval contract cannot serve as tamper-evident gate; governance suite red | eval exit 1; governance 3 fail | Process-integrity risk, not exploit |
| S4 | LOW-MED | crossbeam-epoch 0.9.18 RUSTSEC-2026-0204 (invalid ptr deref in `fmt::Pointer` impl) | cargo audit | Requires unusual code path; fix = dep bump ≥0.9.20 |
| S5 | LOW | anyhow/memmap2 unsound warnings; bincode unmaintained | cargo audit | Track; bump opportunistically |
| S6 | LOW | Test suite mutates tracked fixture (root cause of A1 pollution); uv.lock unreproducible | Task 0.3 carried | Hygiene debt |

## 5. Risk-ranked backlog → proposed Phase B order

1. **B1 (quick wins, ~1 task):** S1 loopback bind guard (default `127.0.0.1`, explicit opt-in flag + warning สำหรับ non-loopback) + S4 dep bump `crossbeam-epoch` + opportunistic S5 bumps
2. **B2:** S3 — re-lock eval contract: verify 9 files' post-lock edits were validator-approved (git history), regenerate manifest hashes, governance suite green
3. **B3:** S2 — wire `AuthPolicy::allows` into dispatch + per-handle capability closure (largest; gates Phase C)
4. then Phase C (brain_* wiring) → D (Z.ai) → E (Console) → F (production closure) ตามแผนเดิม

## 6. Verification commands (for Independent Validator)

```
rustup show                                    # override 1.95-x86_64-pc-windows-msvc
cargo fmt --check                              # exit 0
cargo clippy --all-targets -- -D warnings      # exit 0
cargo audit                                    # 1 vuln (RUSTSEC-2026-0204) + 3 warnings
uv run --no-project --python 3.14.4 python evals/v1/run.py --manifest evals/v1/manifest.json --strict-environment
                                               # exit 1, cases 126/126, byte_lock false (9 files)
cd tests-integration && uv run pytest governance/ -q   # 3 failed, 7 passed
rg -n "0, 0, 0, 0" src/server.rs               # line 85 bind
rg -l "AuthPolicy" src/ | grep -v auth.rs      # empty → not wired
```
