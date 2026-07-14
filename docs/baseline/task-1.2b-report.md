# Task 1.2b — Object Encryption at Rest (DEK/Epoch-KEK Envelope) Report

Status: **BUILDER GREEN — awaiting independent validation**

## Why this task exists

While scoping Task 1.3 ("Policy and concurrency"), its DoD requires a "hard-purge saga crash/retry tests ครบ ledger request, registry replication, **key revocation**, live/object deletion, Git rewrite, index rebuild และ completion" per ADR Decision 7. The object store built in Tasks 0.3/1.1/1.2 stores content as **plaintext**, SHA-256-addressed files — there is no key to revoke. Building a purge saga with a `key_revoked` step on top of that would mean either faking the step (dishonest, and a direct violation of this project's own accuracy-first rule) or silently retrofitting encryption mid-Task-1.3 without a design review. The user chose to insert this as its own task (option C of three presented) rather than either bloating Task 1.3 or building a hollow saga step.

## Scope and TDD evidence

Started from clean canonical HEAD `bea6b86` on `vnext/phase-0` (Task 1.2 validator PASS).

- RED checkpoint commit: `5bbb53d` added `tests/semantic_encryption_v1.rs` (7 tests) plus the `aes-gcm = "0.10"` dependency (`Cargo.toml`/`Cargo.lock`). RED failure was 7 legitimate `E0599` contract errors (`wrapped_key_count`, `destroy_wrapped_key`, `current_epoch`, `rotate_epoch_and_rewrap`, `SemanticError::ObjectUnavailable` all missing); no syntax/config failure.
- GREEN commit: `2b7a1c5` changed `src/semantic.rs`, plus fixes to two pre-existing test files (see "Bug found and fixed" below).

Permitted paths: `Cargo.toml`, `Cargo.lock`, `src/semantic.rs`, `tests/semantic_encryption_v1.rs`, `tests/semantic_vertical_slice.rs` (two assertion fixes only — see below), `docs/baseline/task-1.2b-report.md`.

## Design

- **Object identity is unchanged**: `object_id = sha256:<hex>` of the **plaintext**. This preserves content-addressed dedup and every existing evidence-span byte offset/quote-hash computation (Task 0.3), which all operate on decrypted plaintext exactly as before. Only the bytes stored on disk change.
- **Per-object DEK, per-owner epoch KEK.** Each object gets a random AES-256-GCM data-encryption-key (DEK) at write time; the plaintext is encrypted under that DEK with a random content nonce. The DEK itself is then encrypted ("wrapped") under the owner's current epoch key-encryption-key (KEK) with a random wrap nonce, and the wrapped DEK is stored in a new `wrapped_keys(object_id, epoch, wrapped_dek, dek_nonce)` table. A new `epoch_keys(epoch, key_material, created_at)` table holds the KEK history; `initialize_schema` seeds epoch 1 at store creation, the same pattern already used for the bootstrap client (Task 1.1).
- **On-disk envelope**: `[12-byte content nonce][ciphertext+tag]`. `decrypt_object` reads this, looks up the object's wrapped-key row, unwraps the DEK with the referenced epoch's KEK, decrypts, and only then verifies `sha256(plaintext) == object_id` (an integrity check that now runs on top of AEAD authentication, not instead of it).
- **Atomicity**: `publish_object`'s wrapped-key insert happens in the *same SQL transaction* as the resulting event (both were already open in `mutate_once`), so the two can never diverge on crash. `decrypt_object` is used directly (not through `read_object`) at every call site that already holds an open transaction (`propose`, `propose_inference`, `finish_confirmation`), since opening a second connection while a write transaction is uncommitted would block on SQLite's single WAL writer; `read_object`'s public signature is unchanged for the many call sites with no open transaction.
- **Two new primitives for Task 1.3**:
  - `destroy_wrapped_key(object_id)` — deletes one object's wrapped-DEK row. This is cryptographic erasure of that object alone: its ciphertext bytes may still exist on disk, but nothing can ever unwrap its DEK again. Does not touch the ledger.
  - `rotate_epoch_and_rewrap()` — generates a new epoch KEK, rewraps every surviving object's DEK under it (under the exclusive maintenance write lock, so no concurrent mutation can create a new wrapped-key row under the old epoch mid-rotation), then destroys the old epoch's KEK row. This is the ADR's "rotate the owner encryption epoch and rewrap surviving keys" defense-in-depth half of hard purge: even a previously-compromised old KEK protects nothing once rotation completes.
- **Backup**: no code change needed. `backup_consistent` already copies the whole `semantic.sqlite3` via `VACUUM INTO`, which now naturally includes `epoch_keys`/`wrapped_keys` too. This matches the ADR's own model — a backup taken *before* a key is destroyed remains decryptable (an accepted, documented state that Task 1.3's saga will explicitly invalidate via its `retention_pending` step), while a backup taken *after* destruction cannot decrypt the purged object, since the destroyed key's row is simply absent.

## What the 7 new tests prove

- `object_bytes_on_disk_are_encrypted_not_plaintext` — the on-disk file byte-for-byte does not contain the captured secret anywhere, while the store's own read path still returns the exact plaintext (verified via a downstream evidence-span byte length).
- `identical_content_still_dedupes_to_one_object_and_one_wrapped_key` — dedup is preserved: one object file, one wrapped-key row, for two captures of identical bytes.
- `full_capture_propose_confirm_round_trips_through_decryption` — the existing capture→propose→confirm flow still works end to end through the new encrypt/decrypt layer.
- `tampered_ciphertext_is_rejected_fail_closed` — flipping a byte in the on-disk ciphertext causes a downstream `propose` to fail closed (AEAD authentication failure).
- `destroying_a_wrapped_key_makes_only_that_object_unreadable` — destroying one object's wrapped key breaks only that object; an unrelated object remains fully usable.
- `rotate_epoch_and_rewrap_keeps_existing_objects_readable` — after rotation, the pre-rotation object is still readable (rewrapped) and new writes use the new epoch.
- `backup_consistent_carries_keys_and_stays_decryptable` — a backup taken via `backup_consistent` can independently decrypt objects captured before the backup.

## Bug found and fixed during GREEN

The first implementation added an early-return in `publish_object` for the dedup case (`if destination.exists() { return Ok(object_id); }`) as a micro-optimization to skip redundant encryption work. This broke two **pre-existing, previously-validated Task 0.3 regression tests**:

- `recovery_preserves_shared_objects_and_is_checksum_idempotent` — deliberately re-captures identical content ("deduplicated") in a crash-testing child process to exercise "what happens if a crash occurs mid-dedup." With the early return, the child process's `capture` call short-circuited *before* reaching any `crash_at()` point, so the requested failpoint could never fire — the child completed successfully instead of aborting, and the test's own event-count assertion (`once.events == stable.events`, i.e., 1) failed with `2` since the never-supposed-to-commit second capture committed normally.
- `abrupt_failpoint_matrix_has_atomic_recovery_and_effectively_once_projection`'s `crash_worker` helper — same root cause, observed as the helper's trailing `panic!("configured failpoint did not abort the process")` firing because the configured failpoint was unreachable.

Root cause confirmed by manually invoking the crash-testing child binary directly for every failpoint name via PowerShell (to get an authoritative Windows exit code rather than Git Bash's translated one) and observing consistent early completion instead of the expected `0xC0000409` (`STATUS_STACK_BUFFER_OVERRUN`, the signature Rust's `std::process::abort()` produces on this Windows/MSVC toolchain) once the fix was applied.

**Fix**: removed the early return. Encryption, the file write, and both `crash_at()` points now run unconditionally exactly as the original (pre-encryption) code structure did, even when the destination already exists and the resulting ciphertext will just be discarded on the existing-destination branch a few lines later. This exactly preserves Task 0.3's crash-timing guarantees; the cost is one wasted encrypt-and-discard per true dedup hit, which is bounded and was an intentional, disclosed trade-off in exchange for not silently narrowing an already-validated regression suite's coverage.

## One legitimately-changed test assertion

`confirmation_rejects_a_tampered_evidence_less_proposal_object` (Task 0.3) tampers a proposal object's on-disk bytes and asserts the resulting `confirm` fails. Before this task, tampering (writing arbitrary plaintext JSON over the object file) was caught by a post-decrypt plaintext-checksum mismatch, surfacing as `SemanticError::CorruptLedger`. With encryption, the same tampering is now caught earlier and more strongly — the AEAD authentication tag fails to verify during decryption itself, surfacing as `SemanticError::ObjectUnavailable`, before any plaintext (even wrong plaintext) is ever produced. Updated the assertion accordingly with a comment explaining why. This is a strictly stronger integrity guarantee, not a weakened one.

## Dependencies

Added `aes-gcm = "0.10"` (RustCrypto). Cargo.lock gained 14 new entries, all RustCrypto/well-known pure-Rust crates: `aead`, `aes`, `aes-gcm`, `cipher`, `cpufeatures`, `crypto-common`, `ctr`, `generic-array`, `ghash`, `inout`, `opaque-debug`, `polyval`, `rand_core`, `universal-hash`. None introduce a system/C dependency.

## Verification results

| Gate | Result |
|---|---|
| New suite (`--test semantic_encryption_v1`) | PASS — 8 passed (7 + 1 added for the Independent Validator fix below) |
| Task 1.2 suite (`--test semantic_claims_v1`) | PASS — 12 passed, no regression |
| Task 0.3 suite (`--test semantic_vertical_slice`) | PASS — 18 passed (after the bug fix + 1 updated assertion), no regression in coverage/intent |
| Task 1.1 suite (`--test semantic_store_v1`) | PASS — 11 passed, no regression |
| Rust format | PASS |
| Rust clippy all targets/features (`-D warnings`) | PASS |
| Rust clippy all targets, default features (`-D warnings`) | PASS |
| Rust all targets/features | PASS — 614 passed, 0 failed (606 baseline + 8 new) |
| Semantic per-file coverage (`cargo-llvm-cov 0.8.6`, all four semantic suites) | PASS — 89.30% lines (1,895/2,122), minimum 80% |
| Locked eval (pinned `uv run --python 3.14.4`) | PASS — 126/126, `environment_passed=true`, `thresholds_passed=true` |
| Python governance / engine / mcp / acp | PASS — 10 / 63 / 76 / 26+2 known skips |
| Dependency audit comparison vs `task-0.3-audit-after.json` | PASS — before 4, after 4, zero new findings from the 14 new dependencies |

### Independent Validator finding and fix

First pass returned **PASS** with one non-blocking **MEDIUM** finding: `publish_object`'s dedup check (`destination.exists()`) looked only at file presence, not whether the object still had a *live* wrapped key. If `destroy_wrapped_key` had been called for some content, a later capture of byte-identical content would hit the dedup branch, discard its freshly-generated DEK, and leave the object permanently pointing at ciphertext encrypted under the destroyed (unrecoverable) key — silently, with no error at write time. Not reachable via any wired caller yet, but exactly the primitive Task 1.3's purge saga will call, and the validator recommended resolving it before that wiring happens rather than leaving it as an undocumented surprise.

Fixed in `17d021f`: dedup is now keyed on a live `wrapped_keys` row (checked via a `SELECT` before encryption), not file existence. If the file exists but its key was destroyed, the file is overwritten with a fresh DEK/ciphertext and a fresh wrapped-key row (`INSERT OR REPLACE`) instead of being treated as already stored. A new test, `recapturing_identical_content_after_key_destruction_gets_a_fresh_usable_key`, proves recapture after destruction succeeds with a usable key rather than permanent corruption. The `crash_at()` points remain unconditional exactly as before, so this does not reopen the earlier dedup/crash-test bug.

## Notes and carried risks

- Toolchain and Python-suite invocation follow the established conventions: `cargo +1.95-x86_64-pc-windows-msvc` for every Rust gate; `tests-integration` suites run with CWD = `tests-integration/` and `LLM_WIKI_BIN` pointed at the fresh `CARGO_TARGET_DIR` binary.
- Keys live in the same `semantic.sqlite3` file as everything else (not a separate keystore). This is intentional and matches the ADR's model: encryption here exists for cryptographic erasure (purge), not for protecting a stolen backup from someone who also has the keys — a backup taken before a key is destroyed is expected to remain decryptable, and Task 1.3's purge saga is explicitly responsible for invalidating such backups.
- Existing carried risks (4 RUSTSEC findings, CI action tags, fixture pollution, Windows pagefile sensitivity) remain unchanged and unsuppressed.
- `rotate_epoch_and_rewrap` and `destroy_wrapped_key` are validated here only as standalone primitives; they are not yet wired into any caller-facing capability check or saga state machine — that integration is Task 1.3's job.
