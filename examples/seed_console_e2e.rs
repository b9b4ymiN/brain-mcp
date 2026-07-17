//! Phase E Task E1.4 — seed a persistent `SemanticStore` for Console E2E.
//!
//! WHY THIS EXISTS
//! ---------------
//! The Console HTTP API (`src/api.rs`) only exposes READ and REVIEW routes
//! (search / get / timeline / inbox / evidence / approve / reject / supersede).
//! It deliberately does NOT expose `capture` or `propose` — per §9 the Console
//! app may never write storage directly. That makes it impossible to seed
//! pending proposals for E2E via the HTTP surface.
//!
//! `llm-wiki` has no CLI subcommand for capture/propose/seed either (verified
//! against `src/cli.rs`). So the only path that seeds the SAME store the real
//! `serve` boots against — without adding a runtime `[[bin]]` to the shipping
//! product and without new crate deps — is a Rust *example* binary that links
//! the library and calls `SemanticStore` public methods directly. This is test
//! infrastructure, not Console application code; the "no direct storage write
//! from the Console" rule (§9) is about the Console app, not about a seed tool.
//!
//! An example (under `examples/`) is opt-in: it is NOT built by `cargo build`
//! or `cargo test`, only by `cargo build --example seed_console_e2e` /
//! `cargo run --example seed_console_e2e`. No `[[example]]` entry in Cargo.toml
//! is needed (Cargo auto-discovers `examples/*.rs`).
//!
//! WHAT IT SEEDS
//! -------------
//! The store lives at `<state_dir>/semantic-store` — the exact location
//! `src/server.rs` (lines 418-453) auto-creates / opens when `serve` boots.
//! `state_dir` for the server is `config_path.parent()`. So if Playwright
//! writes its temp TOML to `<tmp>/config.toml`, this seed must target
//! `<tmp>` as `state_dir`; both the seed and the server then touch the same
//! `<tmp>/semantic-store` directory. The store marker pins `allowed_parent`
//! to that same directory, so `SemanticConfig::enabled_for(state_dir)` is the
//! only correct config here.
//!
//! The seed is IDEMPOTENT in operation_id space (capture/propose/confirm are
//! idempotent by `operation_id`): running it twice against a fresh store is
//! the same as running it once; running it twice against an already-seeded
//! store returns IdempotencyConflict for the second pass, which we treat as
//! success (the data is already there). Re-seeding from scratch is done by
//! deleting the store dir, which the Playwright webServer does per run via a
//! fresh tempdir.
//!
//! Seeded dataset (chosen to exercise every Console page):
//!   * Capture+Propose pending:  GULF target_price=58, PTT target_price=62,
//!     AAPL sector="tech", XSS payload (subject+value)
//!   * Capture+Propose+Confirm:  GULF target_price=55 (a prior — feeds timeline
//!     + gives Inbox supersede a prior to replace)
//!
//! Usage:
//!     cargo run --example seed_console_e2e -- <state_dir>

use std::path::PathBuf;
use std::process::ExitCode;

use llm_wiki::semantic::{
    CaptureCommand, ClaimDraft, ConfirmCommand, PrivacyLabel, ProposeCommand, SemanticConfig,
    SemanticStore, TrustedContext,
};
use serde_json::{Value, json};

fn die(msg: impl AsRef<str>) -> ! {
    eprintln!("seed_console_e2e: {}", msg.as_ref());
    std::process::exit(1);
}

/// Builds a `ClaimDraft` for a stock-style claim. The Console is domain-blind
/// (it renders whatever the API returns), but using `domain = "stocks"` keeps
/// the seed realistic and matches the existing Rust test fixtures in
/// `tests/api_console_v1.rs`.
fn draft(subject: &str, predicate: &str, value: Value, domain: &str) -> ClaimDraft {
    ClaimDraft {
        subject: subject.to_owned(),
        predicate: predicate.to_owned(),
        value,
        claim_kind: "external_fact".to_owned(),
        domain: domain.to_owned(),
        confidence_basis_points: 8_000,
        privacy_label: PrivacyLabel::LocalOnly,
        valid_from: None,
        valid_to: None,
    }
}

/// Captures evidence bytes, then proposes. Returns the proposal id.
/// Errors are mapped to strings so the bin can print + exit-1 instead of
/// propagating `?` (the example must NOT use `anyhow` — it would pull a
/// transitive dep the task forbids, even though anyhow IS a workspace dep
/// already; staying on the std error path keeps the example self-contained).
fn seed_proposal(
    store: &SemanticStore,
    context: &TrustedContext,
    op: &str,
    evidence: &str,
    draft: ClaimDraft,
) -> Result<String, String> {
    store
        .capture(
            context,
            CaptureCommand {
                operation_id: format!("{op}-cap"),
                bytes: evidence.as_bytes().to_vec(),
                media_type: "text/plain; charset=utf-8".to_owned(),
            },
        )
        .map_err(|e| format!("capture({op}): {e}"))?;
    let outcome = store
        .propose(
            context,
            ProposeCommand {
                operation_id: op.to_owned(),
                capture_operation_id: format!("{op}-cap"),
                draft,
            },
        )
        .map_err(|e| format!("propose({op}): {e}"))?;
    outcome
        .generated
        .proposal_id
        .map(|id| id.to_string())
        .ok_or_else(|| format!("propose({op}): no proposal_id generated"))
}

/// Capture+propose+confirm. Returns the confirmed claim id (used as a prior
/// for supersede tests). Idempotent by operation_id.
fn seed_confirmed(
    store: &SemanticStore,
    context: &TrustedContext,
    op: &str,
    evidence: &str,
    draft: ClaimDraft,
) -> Result<String, String> {
    seed_proposal(store, context, op, evidence, draft)?;
    let outcome = store
        .confirm(
            context,
            ConfirmCommand {
                operation_id: format!("{op}-confirm"),
                proposal_operation_id: op.to_owned(),
            },
        )
        .map_err(|e| format!("confirm({op}): {e}"))?;
    outcome
        .generated
        .claim_id
        .map(|id| id.to_string())
        .ok_or_else(|| format!("confirm({op}): no claim_id generated"))
}

fn main() -> ExitCode {
    // ── parse args: exactly one positional <state_dir> ──────────────────────
    let mut args = std::env::args_os();
    let _prog = args.next();
    let state_dir: PathBuf = match args.next() {
        Some(s) => PathBuf::from(s),
        None => {
            die(
                "missing required argument <state_dir> (the dir that contains/should contain semantic-store)",
            );
        }
    };
    if args.next().is_some() {
        die("too many arguments; expected exactly one: <state_dir>");
    }
    if !state_dir.exists() {
        die(format!("state_dir does not exist: {}", state_dir.display()));
    }

    let semantic_root = state_dir.join("semantic-store");
    let config = SemanticConfig::enabled_for(&state_dir);

    // Open if the marker is present (created by a prior `serve` boot or seed),
    // otherwise create. Mirrors `src/server.rs` lines 425-453 EXACTLY so the
    // server and the seed share the same store semantics.
    let (store, opened_or_created) = if semantic_root.join("store.marker.json").exists() {
        match SemanticStore::open(&semantic_root, config.clone()) {
            Ok(s) => (s, "opened"),
            Err(e) => die(format!("failed to open {}: {e}", semantic_root.display())),
        }
    } else {
        match SemanticStore::create(&semantic_root, config.clone()) {
            Ok((s, _admin)) => (s, "created"),
            Err(e) => die(format!("failed to create {}: {e}", semantic_root.display())),
        }
    };
    let context = store.trusted_context();

    // ── seed dataset ────────────────────────────────────────────────────────
    //
    // Each row is (op, evidence, draft). operation_ids are stable strings so
    // re-running the seed against an already-seeded store is a no-op-conflict
    // (treated as success below — see the IdempotencyConflict handling).
    //
    // The pending proposals are the ones the Inbox page MUST show; the
    // confirmed prior is what the supersede flow needs (a confirmed claim in
    // scope to replace).

    // Confirmed prior — gives Entity/timeline data AND a supersede target.
    if let Err(e) = seed_confirmed(
        &store,
        &context,
        "e2e-prior-gulf-55",
        "GULF prior target was 55 before the raise.",
        draft("GULF", "target_price", json!(55), "stocks"),
    ) {
        // IdempotencyConflict = already seeded by a prior run; not fatal.
        if !e.contains("IDEMPOTENCY_CONFLICT") {
            die(e);
        }
        eprintln!("seed_console_e2e: prior already confirmed (idempotent) — skipping");
    }

    // Pending proposals — these are what Inbox + Home (recent) must surface.
    let pending: &[(&str, &str, ClaimDraft)] = &[
        (
            "e2e-pending-gulf-58",
            "Analyst note: GULF target raised to 58 on stronger margins.",
            draft("GULF", "target_price", json!(58), "stocks"),
        ),
        (
            "e2e-pending-ptt-62",
            "PTT target price set to 62 reflecting upstream mix.",
            draft("PTT", "target_price", json!(62), "stocks"),
        ),
        (
            "e2e-pending-aapl-sector",
            "AAPL classified under the technology sector.",
            draft("AAPL", "sector", json!("tech"), "stocks"),
        ),
        // XSS payload — DoD #4 (xss-csp.real.spec.ts). MUST render as literal
        // text, never execute. Subject and value both carry payloads so we
        // cover every text-bound field the Console renders.
        // NOTE: seeded as PENDING (not confirmed) — XSS is only asserted in Inbox,
        // not Entity (Entity shows confirmed claims only). Don't "fix" by confirming it.
        (
            "e2e-pending-xss",
            "evidence for the XSS row",
            draft(
                "<img src=x onerror=alert(1)>",
                "target_price",
                json!("<script>alert('xss')</script>"),
                "stocks",
            ),
        ),
    ];
    for (op, evidence, draft) in pending {
        match seed_proposal(&store, &context, op, evidence, draft.clone()) {
            Ok(id) => eprintln!("seed_console_e2e: pending {op} -> {id}"),
            Err(e) => {
                if !e.contains("IDEMPOTENCY_CONFLICT") {
                    die(e);
                }
                eprintln!("seed_console_e2e: pending {op} already seeded (idempotent) — skipping");
            }
        }
    }

    // ── summary: print the pending count so the Playwright webServer log ──
    // shows the seed landed. A non-zero pending count is what unblocks the
    // Inbox review test (DoD #2).
    match store.list_pending_proposals() {
        Ok(list) => {
            eprintln!(
                "seed_console_e2e: store {opened_or_created} at {}; {} pending proposal(s)",
                semantic_root.display(),
                list.len()
            );
            for p in &list {
                eprintln!(
                    "  pending: {} {}={} ({})",
                    p.subject, p.predicate, p.value, p.domain
                );
            }
        }
        Err(e) => die(format!("list_pending_proposals: {e}")),
    }

    ExitCode::SUCCESS
}
