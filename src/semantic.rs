//! Isolated semantic authority spike.
//!
//! This module deliberately has no dependency on the wiki write path or any
//! projection used by the existing runtime. SQLite is the sole semantic
//! transition authority; the JSON snapshot is replaceable and recoverable.

use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use chrono::{DateTime, Utc};
#[cfg(feature = "semantic-test-failpoints")]
use parking_lot::Condvar;
use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

// Phase E Task E3.1: trust/operations producers surface the contract types
// defined in `crate::trust` (TrustFlag, RetrievalTrace, JobSummary, BackupHealth,
// ClientActivity, EvalSummary). Imported here so the SemanticStore producers
// below return the contract types directly.
use crate::trust::{
    BackupHealth, ClientActivity, EvalSummary, JobSummary, RetrievalTrace, TrustFlag,
};

const DATABASE_FILE: &str = "semantic.sqlite3";
const MARKER_FILE: &str = "store.marker.json";
const PROJECTION_FILE: &str = "projection.json";
const OBJECT_MEDIA_TYPE: &str = "application/vnd.brain.semantic+json";
const DEFAULT_MAX_OBJECT_BYTES: u64 = 32 * 1024 * 1024;
const AES_GCM_NONCE_LEN: usize = 12;
/// AES-256 raw key length, in bytes. Used by [`SemanticStore::load_or_create_backup_key`]
/// (Task F3.1) to validate a persisted `backup.key` and by [`Aes256Gcm`]
/// everywhere else — kept as a named constant rather than a literal so the
/// 32-byte contract is one-line-auditable.
const AES_256_KEY_LEN: usize = 32;
/// Operator-managed encrypted-backup key file (Task F3.1). 32 raw bytes
/// dropped at `<store_root>/backup.key`. MUST be backed up separately from
/// the encrypted backups themselves (chicken-and-egg: a backup encrypted
/// under a key it also contains is unrecoverable).
const BACKUP_KEY_FILE: &str = "backup.key";
/// Encrypted-backup manifest file (Task F3.1). Plaintext JSON written into
/// every encrypted-backup target dir so operators can inspect layer list +
/// composite checksum + created-at without decrypting first.
const BACKUP_MANIFEST_FILE: &str = "manifest.json";
/// `manifest.json` schema version (Task F3.1). Bumped on incompatible
/// changes to the manifest shape.
const BACKUP_MANIFEST_VERSION: u32 = 1;
/// Cipher identifier recorded in `manifest.json`. AES-256-GCM, per the
/// 2026-07-18 user decision to "follow master plan" for backup encryption.
const BACKUP_CIPHER_ALG: &str = "AES-256-GCM";
const BOOTSTRAP_CLIENT_LABEL: &str = "__bootstrap__";
/// Capability granted to the bootstrap identity and to any client created
/// via the unscoped `register_client`, preserving the pre-Task-1.3 behavior
/// that every registered client could confirm/reject/retract/supersede.
/// A restricted (e.g. AI extraction worker, ADR Decision 8) identity must
/// be created via `register_client_scoped` with a narrower list instead.
/// `"propose"` was added in Phase D Task D2 (F2): it gates
/// `propose_inference` so a scoped-to-`["propose"]` worker identity can
/// submit inference proposals but not confirm/supersede/purge — the
/// bootstrap/default set keeps it alongside confirm+purge so the owner path
/// is unaffected (it already had implicit access via the prior `None` gate).
const DEFAULT_CLIENT_CAPABILITIES: &[&str] = &["confirm", "purge", "propose"];
const VALID_CLIENT_CAPABILITIES: &[&str] = &["confirm", "purge", "propose"];

/// On-disk schema version. Bumped 1 → 2 in Task 2.2 when the entity tables
/// (`entities`, `entity_aliases`) and the `claim_status.entity_id` column
/// were added. Bumped 2 → 3 in Task F3.3 — the bump is the proof-of-path for
/// the schema upgrade runner (`plan_schema_upgrade` +
/// `execute_schema_upgrade` + `rollback_schema_upgrade`); the v2→v3 migration
/// itself is a noop placeholder (every step is a reversible noop), so a
/// store upgraded from 2 → 3 has identical ledger_head + purge_epoch (the
/// composite_checksum's third input — `schema_version` — DOES change, so the
/// checksum value differs across the bump; tests assert data-intact via
/// ledger_head + purge_epoch equality, not raw checksum equality).
///
/// A store created under an older schema_version that has a known migration
/// path (today only 2 → 3) refuses to serve until an operator runs
/// `llm-wiki recovery upgrade`; an older version with NO migration path
/// still fails closed (see `validate_database_identity`).
///
/// NOTE: this is the *on-disk DDL* version, distinct from the *event wire*
/// version stamped on each `EventEnvelope.schema_version`. The wire format
/// of an event has not changed in Task 2.2 (same `EventEnvelope` JSON
/// shape), so events keep `schema_version: 1` to stay valid against the
/// hash-locked `event-schema-v1.json` contract. Only the on-disk schema
/// (marker + DDL) moved to 3.
pub const CURRENT_DISK_SCHEMA_VERSION: u8 = 3;
/// Event wire-format version (unchanged since the schema was hash-locked in
/// Task 0.2). Kept as a named constant rather than a literal so the next
/// genuine wire break is a one-line change with a clear audit trail.
const CURRENT_EVENT_SCHEMA_VERSION: u8 = 1;

/// Clock boundary used to make transaction-time behavior testable.
pub trait SemanticClock: Send + Sync + Debug {
    /// Returns the server-observed UTC time for the next event.
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug)]
struct SystemClock;

impl SemanticClock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Runtime guard and local-store boundary. Disabled is the default.
#[derive(Clone, Debug)]
pub struct SemanticConfig {
    /// Explicit runtime guard. A disabled call returns before filesystem work.
    pub enabled: bool,
    allowed_parent: Option<PathBuf>,
    clock: Arc<dyn SemanticClock>,
    max_object_bytes: u64,
    purge_registry_targets: Vec<PathBuf>,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_parent: None,
            clock: Arc::new(SystemClock),
            max_object_bytes: DEFAULT_MAX_OBJECT_BYTES,
            purge_registry_targets: Vec::new(),
        }
    }
}

impl SemanticConfig {
    /// Enables an isolated store below an explicitly supplied parent.
    pub fn enabled_for(allowed_parent: impl AsRef<Path>) -> Self {
        Self {
            enabled: true,
            allowed_parent: Some(allowed_parent.as_ref().to_path_buf()),
            clock: Arc::new(SystemClock),
            max_object_bytes: DEFAULT_MAX_OBJECT_BYTES,
            purge_registry_targets: Vec::new(),
        }
    }

    /// Replaces the system clock; intended for deterministic application tests.
    pub fn with_clock(mut self, clock: Arc<dyn SemanticClock>) -> Self {
        self.clock = clock;
        self
    }

    /// Overrides the capture size limit; zero is rejected at create/open.
    pub fn with_max_object_bytes(mut self, limit: u64) -> Self {
        self.max_object_bytes = limit;
        self
    }

    /// Sets the independent purge-registry replication targets (ADR Decision
    /// 7: "replicate to at least two independent targets and require quorum
    /// acknowledgement"). `append_registry_denial` requires at least 2;
    /// `open` seals the store if a reachable quorum of these disagrees with
    /// (is ahead of) the local copy.
    pub fn with_purge_registry_targets(mut self, targets: Vec<PathBuf>) -> Self {
        self.purge_registry_targets = targets;
        self
    }
}

/// Errors fail closed and do not expose object content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticError {
    Disabled,
    InvalidRoot(String),
    MarkerMismatch,
    ActiveHandles,
    IdempotencyConflict,
    InvalidInterval,
    InvalidCapture(String),
    InvalidClaim(String),
    InvalidTransition(String),
    UnsupportedInference,
    ObjectUnavailable(String),
    CapabilityDenied(String),
    MissingDependency(String),
    CorruptLedger(String),
    Io(String),
    Database(String),
    DatabaseContention(String),
    Serialization(String),
    Denied(String),
    RegistrySealed(String),
    RegistryQuorumFailed(String),
}

impl fmt::Display for SemanticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("semantic store is disabled"),
            Self::InvalidRoot(reason) => write!(formatter, "invalid semantic root: {reason}"),
            Self::MarkerMismatch => {
                formatter.write_str("semantic store marker does not match capability")
            }
            Self::ActiveHandles => {
                formatter.write_str("semantic store has active handles or transactions")
            }
            Self::IdempotencyConflict => formatter.write_str("IDEMPOTENCY_CONFLICT"),
            Self::InvalidInterval => {
                formatter.write_str("valid interval must be non-empty and ordered")
            }
            Self::InvalidCapture(reason) => write!(formatter, "invalid capture: {reason}"),
            Self::InvalidClaim(reason) => write!(formatter, "invalid claim: {reason}"),
            Self::InvalidTransition(reason) => write!(formatter, "invalid transition: {reason}"),
            Self::UnsupportedInference => formatter.write_str(
                "cannot confirm an inference proposal with no evidence; accept it by creating a new user_assertion or decision claim instead",
            ),
            Self::ObjectUnavailable(object_id) => write!(
                formatter,
                "object {object_id} cannot be decrypted (destroyed key or tampered ciphertext)"
            ),
            Self::CapabilityDenied(reason) => write!(formatter, "capability denied: {reason}"),
            Self::MissingDependency(value) => {
                write!(formatter, "required semantic input is missing: {value}")
            }
            Self::CorruptLedger(value) => {
                write!(formatter, "semantic ledger validation failed: {value}")
            }
            Self::Io(value) => write!(formatter, "semantic filesystem error: {value}"),
            Self::Database(value) => write!(formatter, "semantic database error: {value}"),
            Self::DatabaseContention(value) => {
                write!(formatter, "semantic database contention: {value}")
            }
            Self::Serialization(value) => {
                write!(formatter, "semantic serialization error: {value}")
            }
            Self::Denied(id) => {
                write!(formatter, "{id} is denied by the purge registry")
            }
            Self::RegistrySealed(reason) => {
                write!(formatter, "purge registry is sealed: {reason}")
            }
            Self::RegistryQuorumFailed(reason) => {
                write!(formatter, "purge registry replication quorum failed: {reason}")
            }
        }
    }
}

impl std::error::Error for SemanticError {}

type Result<T> = std::result::Result<T, SemanticError>;

fn io_error(error: impl fmt::Display) -> SemanticError {
    SemanticError::Io(error.to_string())
}

fn database_error(error: rusqlite::Error) -> SemanticError {
    let contention = error.sqlite_error_code().is_some_and(|code| {
        matches!(
            code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        )
    }) || matches!(
        &error,
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.extended_code == rusqlite::ffi::SQLITE_PROTOCOL
    );
    if contention {
        SemanticError::DatabaseContention(error.to_string())
    } else {
        SemanticError::Database(error.to_string())
    }
}

fn serialization_error(error: impl fmt::Display) -> SemanticError {
    SemanticError::Serialization(error.to_string())
}

/// Canonical RFC 8785 JSON bytes.
pub fn canonicalize_json(value: &Value) -> Result<Vec<u8>> {
    serde_jcs::to_vec(value).map_err(serialization_error)
}

fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_jcs::to_vec(value).map_err(serialization_error)
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureCommand {
    pub operation_id: String,
    pub bytes: Vec<u8>,
    pub media_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimDraft {
    pub subject: String,
    pub predicate: String,
    pub value: Value,
    pub claim_kind: String,
    pub domain: String,
    pub confidence_basis_points: u16,
    pub privacy_label: PrivacyLabel,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyLabel {
    LocalOnly,
    PrivateExternalAllowed,
    Publishable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeCommand {
    pub operation_id: String,
    pub capture_operation_id: String,
    pub draft: ClaimDraft,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmCommand {
    pub operation_id: String,
    pub proposal_operation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeInferenceCommand {
    pub operation_id: String,
    pub evidence_capture_operation_ids: Vec<String>,
    pub method: String,
    pub model: Option<String>,
    pub prompt_version: Option<String>,
    /// Subject validator version that screened this proposal's subject.
    /// `None` for legacy proposals (pre-Phase 1.5) or when validation skipped.
    /// `#[serde(default)]` keeps backward compat: historical JSON without
    /// this field deserializes to None instead of failing under
    /// `deny_unknown_fields`.
    #[serde(default)]
    pub subject_validator_version: Option<String>,
    pub draft: ClaimDraft,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RejectCommand {
    pub operation_id: String,
    pub proposal_operation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetractCommand {
    pub operation_id: String,
    pub claim_operation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupersedeCommand {
    pub operation_id: String,
    pub proposal_operation_id: String,
    pub superseded_claim_operation_ids: Vec<String>,
}

/// Confirms a proposal by `proposal_id` rather than the proposer's
/// `operation_id` (Task 5.1 Inbox). Unlike [`ConfirmCommand`], this does not
/// require the calling context to be the same client that proposed — any
/// client holding the `confirm` capability may act on it, and the resulting
/// `claim_confirmed` event's actor/client reflect the *reviewer*, not the
/// original proposer (correct audit trail for a review workflow).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmByProposalIdCommand {
    pub operation_id: String,
    pub proposal_id: Uuid,
}

/// Rejects a proposal by `proposal_id`. See [`ConfirmByProposalIdCommand`]
/// for why this exists alongside [`RejectCommand`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RejectByProposalIdCommand {
    pub operation_id: String,
    pub proposal_id: Uuid,
}

/// Supersedes prior claims by `claim_id` while confirming a proposal by
/// `proposal_id`. See [`ConfirmByProposalIdCommand`] for why this exists
/// alongside [`SupersedeCommand`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupersedeByProposalIdCommand {
    pub operation_id: String,
    pub proposal_id: Uuid,
    pub superseded_claim_ids: Vec<Uuid>,
}

/// Propose a claim the owner asserted directly (Task 2.2 — human edit becomes
/// an authored event). The `utterance` bytes ARE the evidence: there is no
/// external source span, so this path mints a `Provenance::UserAssertion`
/// rather than `Provenance::Evidence`. ADR Decision 6.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeUserAssertionCommand {
    pub operation_id: String,
    pub utterance: Vec<u8>,
    pub draft: ClaimDraft,
}

/// Propose a mechanically-derived claim (Task 2.2). The derivation is
/// reproducible from `input_hashes` + `method`/`method_version`, and the
/// `output_hash` pins the derived value. ADR Decision 6's `mechanical`
/// variant; §5 Memory Policy permits auto-confirm for verifiable metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeMechanicalCommand {
    pub operation_id: String,
    pub method: String,
    pub method_version: String,
    pub input_hashes: Vec<String>,
    pub output_hash: String,
    pub draft: ClaimDraft,
}

/// Rename an entity's canonical subject (Task 2.2 DoD bullet 3). The old
/// subject becomes a `former_subject` alias so existing references still
/// resolve — stable IDs and backlinks are preserved.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameEntityCommand {
    pub operation_id: String,
    pub entity_id: Uuid,
    pub new_subject: String,
}

/// Merge the source entity into the target (Task 2.2 DoD bullet 3). Every
/// claim attached to the source is rewritten onto the target, and the
/// source's subject (plus its prior aliases) become backlinks to the target.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeEntitiesCommand {
    pub operation_id: String,
    pub source_entity_id: Uuid,
    pub target_entity_id: Uuid,
}

/// One predicate → target assignment inside a [`SplitCommand`] (Phase E Task
/// E3.1). Claims on the source entity whose predicate matches `predicate` are
/// rewritten onto `target_entity_id`. Predicates not listed stay on the source.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PredicateAssignment {
    pub predicate: String,
    pub target_entity_id: Uuid,
}

/// Split the source entity by predicate (Phase E Task E3.1 — the mirror of
/// [`SemanticStore::merge_entities`]). For each claim attached to the source
/// entity whose predicate matches one in `assignments`, the claim's
/// `entity_id` is rewritten onto that assignment's target. Predicates not in
/// `assignments` stay on the source. The source entity is NOT deleted: it
/// retains the residual claims, and the operation is fully reversible via
/// `retract` on the emitted `entity_split` event.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitCommand {
    pub operation_id: String,
    pub source_entity_id: Uuid,
    pub assignments: Vec<PredicateAssignment>,
}

/// One claim moved from source to a target during a split.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MovedClaim {
    pub claim_id: Uuid,
    pub predicate: String,
    pub from_entity_id: Uuid,
    pub to_entity_id: Uuid,
}

/// Result of [`SemanticStore::split_entities`]. `source_remaining_claim_count`
/// is the number of claims still attached to the source after the split;
/// `moved_claims` enumerates every rewritten claim; `event` is the emitted
/// `entity_split` audit event (reversible via retract).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SplitOutcome {
    pub source_remaining_claim_count: u64,
    pub moved_claims: Vec<MovedClaim>,
    pub event: EventEnvelope,
}

/// Status of an in-process async job tracked by [`SemanticStore::register_job`]
/// (Phase E Task E3.1). The registry is in-memory and per-store-instance: it
/// exists so [`SemanticStore::job_summary`] can surface a non-zero queue once
/// Phase E publishers wire in. For E3.1 itself there are no callers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobRecord {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
}

/// One row of the `entities` table (Task 2.2). Stable UUIDv7 identity that
/// survives rename/merge; `canonical_subject` is the current display name.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntityRecord {
    pub entity_id: Uuid,
    pub domain: String,
    pub canonical_subject: String,
    pub created_at: DateTime<Utc>,
}

/// Outcome of one row examined by [`SemanticStore::backfill_entity_ids`]
/// (Task 2.3). `outcome` is one of `migrated`, `skipped`, `ambiguous`,
/// `error` — every claim_status row produces exactly one record, so the
/// migration report accounts for the whole table.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationRecord {
    pub claim_id: Uuid,
    pub domain: String,
    pub subject: String,
    pub predicate: String,
    pub outcome: String,
    /// Populated when `outcome == "migrated"`; the entity_id written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<Uuid>,
    /// Human-readable reason for `ambiguous`/`error` outcomes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Aggregate report for [`SemanticStore::backfill_entity_ids`] (Task 2.3).
/// `records` has one entry per examined row, and the four counts sum to
/// `records.len()`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationReport {
    pub migrated: usize,
    pub skipped: usize,
    pub ambiguous: usize,
    pub error: usize,
    pub records: Vec<MigrationRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectPayload {
    pub kind: String,
    pub object_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventEnvelope {
    pub schema_version: u8,
    pub event_id: Uuid,
    pub owner_id: Uuid,
    pub event_seq: u64,
    pub event_type: String,
    pub recorded_at: DateTime<Utc>,
    pub actor_id: Uuid,
    pub client_id: Uuid,
    pub operation_id: String,
    pub request_hash: String,
    pub payload: ObjectPayload,
    pub prior_event_hash: Option<String>,
    pub event_hash: String,
    pub purge_epoch: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeneratedIds {
    pub source_id: Option<Uuid>,
    pub rendition_id: Option<Uuid>,
    pub evidence_id: Option<Uuid>,
    pub proposal_id: Option<Uuid>,
    pub claim_id: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MutationOutcome {
    pub event: EventEnvelope,
    pub generated: GeneratedIds,
}

impl MutationOutcome {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        canonical_bytes(self)
    }
}

/// Origin classification for a claim (GOAL-vNext §13 Task 2.2 DoD bullet 1).
/// Derived deterministically from provenance: anything an authenticated owner
/// asserted or that was mechanically verified is `HumanAuthored`; anything an
/// AI worker proposed (evidence ingest or inference) is `AgentPropored`. The
/// `generated` ownership class is a *file* property of the projection output,
/// not a claim origin — see `projection::OWNERSHIP_MARKER`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OriginClass {
    HumanAuthored,
    AgentProposed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClaimView {
    pub claim_id: Uuid,
    pub proposal_id: Uuid,
    pub subject: String,
    pub predicate: String,
    pub value: Value,
    pub claim_kind: String,
    pub status: String,
    pub domain: String,
    pub confidence_basis_points: u16,
    pub privacy_label: PrivacyLabel,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
    pub confirmed_event_seq: u64,
    /// ADR Decision 6 provenance variant name (`evidence` / `inference` /
    /// `user_assertion` / `mechanical`). Exposed so projection adapters and
    /// the generated-wiki frontmatter can render provenance without
    /// re-deriving it from the encrypted claim payload.
    pub provenance_kind: String,
    /// Task 2.2 origin class — derived from `provenance_kind`. See
    /// [`OriginClass`].
    pub origin: OriginClass,
    /// Stable UUIDv7 entity this claim resolves to (ADR Decision 3). Populated
    /// for confirmed claims; `None` only for legacy rows that pre-date the
    /// entity table (none exist in production — backfill is Task 2.3).
    pub entity_id: Option<Uuid>,
}

/// Result of a scoped, time-aware claim query. `active` claims are current
/// at `world_time` as of the requested `ledger_head`; `future` claims are
/// confirmed and not superseded/retracted but not yet valid (`valid_from` is
/// ahead of `world_time`); `past` claims are superseded, retracted, or
/// expired (`valid_to` at/before `world_time`) as of that same ledger head.
/// More than one `active` claim means the scope is currently disputed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct CurrentClaims {
    pub active: Vec<ClaimView>,
    pub future: Vec<ClaimView>,
    pub past: Vec<ClaimView>,
}

/// A pending proposal awaiting review (Task 5.1 Inbox). Read-only summary of
/// a `claim_proposed` event whose `proposal_status` is still `proposed`.
/// Store-scoped like [`ClaimView`] — not tied to whichever client proposed
/// it, so the Console (a different client) can list and act on it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProposalSummary {
    pub proposal_id: Uuid,
    pub domain: String,
    pub subject: String,
    pub predicate: String,
    pub value: Value,
    pub claim_kind: String,
    /// ADR Decision 6 provenance variant name (`evidence` / `inference` /
    /// `user_assertion` / `mechanical`).
    pub provenance_kind: String,
    pub submitted_at: DateTime<Utc>,
    pub event_seq: u64,
}

/// Human-readable evidence for the Inbox diff view (Task 5.1 "แสดง
/// evidence/diff ก่อน commit"). `excerpt` is `None` for provenance kinds
/// that carry no raw text (e.g. `mechanical`, or an `inference` proposed
/// with `unsupported=true` and no evidence spans).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EvidenceSummary {
    pub provenance_kind: String,
    pub excerpt: Option<String>,
    pub source_id: Option<Uuid>,
    pub quote_hash: Option<String>,
    /// Phase 1.6 — true iff the proposal value was found in the excerpt.
    #[serde(default)]
    pub value_located: bool,
    /// Phase 1.6 — char offset of the value within `excerpt` (None if `!value_located`).
    #[serde(default)]
    pub value_offset: Option<usize>,
    /// Phase 1.6 — char length of the value within `excerpt` (None if `!value_located`).
    #[serde(default)]
    pub value_len: Option<usize>,
    /// Phase 1.6 — true if the excerpt was windowed/clamped.
    #[serde(default)]
    pub excerpt_truncated: bool,
    /// Phase 1.6 — rendition IDs of non-primary spans (collapsed "N more sources" UI).
    #[serde(default)]
    pub additional_sources: Vec<String>,
}

/// Result of `purge_preview`: the caller must echo `preview_hash` and
/// `nonce` back to `purge_execute` before `expires_at` (ADR Decision 7).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PurgePreview {
    pub preview_hash: String,
    pub nonce: String,
    pub expires_at: DateTime<Utc>,
    pub targets: Vec<String>,
}

/// Current state/result of a hard-purge saga. `state` is one of
/// `requested`, `registry_denied`, `key_revoked`, `live_deleted`,
/// `projections_cleaned`, `retention_pending`, or `completed`; only a
/// `completed` receipt carries a `composite_checksum`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PurgeReceipt {
    pub purge_id: Uuid,
    pub state: String,
    pub registry_epoch: Option<u64>,
    pub new_backup_path: Option<String>,
    pub composite_checksum: Option<String>,
}

struct PurgeSagaRow {
    state: String,
    targets: Vec<String>,
    registry_epoch: Option<u64>,
    new_backup_path: Option<String>,
    composite_checksum: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectionState {
    pub event_count: usize,
    pub ledger_head: u64,
    pub checksum: String,
}

impl ProjectionState {
    pub fn from_diagnostics(value: &StoreDiagnostics) -> Self {
        Self {
            event_count: value.events,
            ledger_head: value.event_sequences.last().copied().unwrap_or(0),
            checksum: value.ledger_checksum.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreDiagnostics {
    pub events: usize,
    pub operations: usize,
    pub outbox_pending: usize,
    pub objects: usize,
    pub event_sequences: Vec<u64>,
    pub ledger_checksum: String,
    pub projection_checksum: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoragePragmas {
    pub journal_mode: String,
    pub synchronous: i64,
    pub foreign_keys: i64,
    pub busy_timeout_ms: i64,
}

impl StoreDiagnostics {
    pub fn empty() -> Self {
        let checksum = sha256(&[]);
        Self {
            events: 0,
            operations: 0,
            outbox_pending: 0,
            objects: 0,
            event_sequences: Vec::new(),
            ledger_checksum: checksum.clone(),
            projection_checksum: checksum,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedContext {
    store_uuid: Uuid,
    owner_id: Uuid,
    actor_id: Uuid,
    client_id: Uuid,
}

impl TrustedContext {
    /// The store this context is scoped to. Exposed so callers (e.g. the
    /// Console API layer, Task 5.1) can display or audit which identity a
    /// session is acting as without reaching into store internals.
    pub fn owner_id(&self) -> Uuid {
        self.owner_id
    }

    pub fn actor_id(&self) -> Uuid {
        self.actor_id
    }

    pub fn client_id(&self) -> Uuid {
        self.client_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreMarker {
    schema_version: u8,
    store_uuid: Uuid,
    owner_id: Uuid,
    actor_id: Uuid,
    client_id: Uuid,
    deletion_nonce: Uuid,
    allowed_parent: String,
}

#[derive(Debug)]
struct RootCoordinator {
    maintenance: RwLock<()>,
    writer: Mutex<()>,
    projection: Mutex<()>,
    active_handles: AtomicUsize,
    active_transactions: AtomicUsize,
    #[cfg(feature = "semantic-test-failpoints")]
    recovery_waiting: AtomicUsize,
    #[cfg(feature = "semantic-test-failpoints")]
    pause_read_after_deny_check: Arc<PauseState>,
}

impl RootCoordinator {
    fn new() -> Self {
        Self {
            maintenance: RwLock::new(()),
            writer: Mutex::new(()),
            projection: Mutex::new(()),
            active_handles: AtomicUsize::new(0),
            active_transactions: AtomicUsize::new(0),
            #[cfg(feature = "semantic-test-failpoints")]
            recovery_waiting: AtomicUsize::new(0),
            #[cfg(feature = "semantic-test-failpoints")]
            pause_read_after_deny_check: Arc::default(),
        }
    }
}

static COORDINATORS: OnceLock<Mutex<HashMap<PathBuf, Weak<RootCoordinator>>>> = OnceLock::new();

fn coordinator_for(root: &Path) -> Arc<RootCoordinator> {
    let registry = COORDINATORS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = registry.lock();
    if let Some(existing) = registry.get(root).and_then(Weak::upgrade) {
        return existing;
    }
    let coordinator = Arc::new(RootCoordinator::new());
    registry.insert(root.to_path_buf(), Arc::downgrade(&coordinator));
    coordinator
}

pub struct SemanticStore {
    root: PathBuf,
    marker: StoreMarker,
    coordinator: Arc<RootCoordinator>,
    clock: Arc<dyn SemanticClock>,
    max_object_bytes: u64,
    purge_registry_targets: Vec<PathBuf>,
    /// In-process async-job registry (Phase E Task E3.1). Holds every job
    /// registered via [`Self::register_job`] until it is completed or failed.
    /// Empty by default — no Phase E publisher wires into it yet, so
    /// [`Self::job_summary`] reports `{active:0, queued:0, failed:0}` until
    /// they do. Per-instance (not persisted): a restart forgets in-flight
    /// jobs, matching the dashboard's "current queue" semantics.
    jobs: Arc<Mutex<HashMap<String, JobRecord>>>,
    #[cfg(feature = "semantic-test-failpoints")]
    pause_after_commit: Arc<PauseState>,
}

impl Debug for SemanticStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SemanticStore")
            .field("capability", &"<redacted>")
            .finish()
    }
}

impl Drop for SemanticStore {
    fn drop(&mut self) {
        self.coordinator
            .active_handles
            .fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct StoreAdmin {
    root: PathBuf,
    allowed_parent: PathBuf,
    store_uuid: Uuid,
    deletion_nonce: Uuid,
    coordinator: Arc<RootCoordinator>,
}

impl Debug for StoreAdmin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreAdmin")
            .field("capability", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackStatus {
    Removed,
    AlreadyRemoved,
}

#[derive(Debug, Clone, Copy)]
pub struct ManualRecovery;

#[cfg(feature = "semantic-test-failpoints")]
#[derive(Debug, Default)]
struct PauseState {
    flags: Mutex<PauseFlags>,
    condvar: Condvar,
}

#[cfg(feature = "semantic-test-failpoints")]
#[derive(Debug, Default)]
struct PauseFlags {
    armed: bool,
    entered: bool,
    released: bool,
}

/// Test-only handle that holds one mutation paused after its database commit
/// while the maintenance read guard is still held, so tests can queue
/// recovery behind it and prove the pair converges without deadlocking.
#[cfg(feature = "semantic-test-failpoints")]
pub struct SemanticTestPause {
    state: Arc<PauseState>,
}

#[cfg(feature = "semantic-test-failpoints")]
impl SemanticTestPause {
    pub fn wait_until_entered(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut flags = self.state.flags.lock();
        while !flags.entered {
            if self
                .state
                .condvar
                .wait_until(&mut flags, deadline)
                .timed_out()
            {
                return flags.entered;
            }
        }
        true
    }

    pub fn release(&self) {
        let mut flags = self.state.flags.lock();
        flags.released = true;
        self.state.condvar.notify_all();
    }
}

#[cfg(feature = "semantic-test-failpoints")]
impl Drop for SemanticTestPause {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(feature = "semantic-test-failpoints")]
#[derive(Debug)]
pub struct SemanticTestTransaction {
    connection: Connection,
    coordinator: Arc<RootCoordinator>,
}

#[cfg(feature = "semantic-test-failpoints")]
impl Drop for SemanticTestTransaction {
    fn drop(&mut self) {
        let _ = self.connection.execute_batch("ROLLBACK");
        self.coordinator
            .active_transactions
            .fetch_sub(1, Ordering::SeqCst);
    }
}

struct ActiveTransaction<'a> {
    coordinator: &'a RootCoordinator,
}

impl<'a> ActiveTransaction<'a> {
    fn new(coordinator: &'a RootCoordinator) -> Self {
        coordinator
            .active_transactions
            .fetch_add(1, Ordering::SeqCst);
        Self { coordinator }
    }
}

impl Drop for ActiveTransaction<'_> {
    fn drop(&mut self) {
        self.coordinator
            .active_transactions
            .fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct MutationMaterial {
    event_type: &'static str,
    object_bytes: Vec<u8>,
    media_type: String,
    generated: GeneratedIds,
}

#[derive(Clone, Copy, Debug)]
struct EventIdentity {
    event_id: Uuid,
    event_seq: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Provenance {
    Evidence {
        source_id: Uuid,
        rendition_id: Uuid,
        evidence_id: Uuid,
        object_id: String,
        byte_start: u64,
        byte_end: u64,
        quote_hash: String,
    },
    Inference {
        method: String,
        model: Option<String>,
        prompt_version: Option<String>,
        evidence: Vec<InferenceEvidenceSpan>,
        unsupported: bool,
    },
    /// A claim the owner asserted directly — "human edit becomes an authored
    /// event" (GOAL-vNext §13 Task 2.2 DoD bullet 2). The evidence is the
    /// utterance itself, captured as an object; there is no external source
    /// span. ADR Decision 6 names this variant and §5 Memory Policy marks
    /// explicit "จำไว้" / preference saves as `user_assertion`.
    UserAssertion {
        actor_id: Uuid,
        utterance_object_id: String,
        utterance_byte_start: u64,
        utterance_byte_end: u64,
    },
    /// Mechanically-derived metadata (hash/title/time) whose derivation is
    /// reproducible from input hashes — ADR Decision 6's fourth variant. §5
    /// allows auto-confirm for "metadata ที่ตรวจเชิงกลไกได้".
    Mechanical {
        method: String,
        method_version: String,
        input_hashes: Vec<String>,
        output_hash: String,
    },
}

impl Provenance {
    /// The ADR Decision 6 variant name, snake_cased — exposed on `ClaimView`
    /// so projection adapters (and the generated-wiki frontmatter) can tell
    /// provenance kinds apart without re-deriving them.
    fn kind(&self) -> &'static str {
        match self {
            Provenance::Evidence { .. } => "evidence",
            Provenance::Inference { .. } => "inference",
            Provenance::UserAssertion { .. } => "user_assertion",
            Provenance::Mechanical { .. } => "mechanical",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InferenceEvidenceSpan {
    source_id: Uuid,
    rendition_id: Uuid,
    evidence_id: Uuid,
    object_id: String,
    byte_start: u64,
    byte_end: u64,
    quote_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProposalObject {
    kind: String,
    proposal_id: Uuid,
    source_object_id: String,
    provenance: Provenance,
    draft: ClaimDraft,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConfirmationObject {
    kind: String,
    claim: ClaimRecord,
}

#[derive(Debug, Serialize, Deserialize)]
struct RejectionObject {
    kind: String,
    proposal_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
struct RetractionObject {
    kind: String,
    claim_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClaimRecord {
    claim_id: Uuid,
    proposal_id: Uuid,
    subject: String,
    predicate: String,
    value: Value,
    claim_kind: String,
    status: String,
    domain: String,
    confidence_basis_points: u16,
    privacy_label: PrivacyLabel,
    valid_from: Option<DateTime<Utc>>,
    valid_to: Option<DateTime<Utc>>,
    recorded_event_id: Uuid,
    recorded_event_seq: u64,
    provenance: Provenance,
    supersedes: Vec<Uuid>,
    retracts: Vec<Uuid>,
}

impl SemanticStore {
    pub fn create(root: impl AsRef<Path>, config: SemanticConfig) -> Result<(Self, StoreAdmin)> {
        let (requested_root, allowed_parent) = validate_requested_root(root.as_ref(), &config)?;
        validate_object_limit(&config)?;
        if requested_root.exists() {
            return Err(SemanticError::InvalidRoot(
                "target already exists".to_owned(),
            ));
        }
        fs::create_dir(&requested_root).map_err(io_error)?;
        fs::create_dir(requested_root.join("objects")).map_err(io_error)?;
        fs::create_dir(requested_root.join("staging")).map_err(io_error)?;
        let canonical_root = requested_root.canonicalize().map_err(io_error)?;

        let marker = StoreMarker {
            schema_version: CURRENT_DISK_SCHEMA_VERSION,
            store_uuid: Uuid::now_v7(),
            owner_id: Uuid::now_v7(),
            actor_id: Uuid::now_v7(),
            client_id: Uuid::now_v7(),
            deletion_nonce: Uuid::now_v7(),
            allowed_parent: allowed_parent.to_string_lossy().into_owned(),
        };
        write_new_file(
            &canonical_root.join(MARKER_FILE),
            &canonical_bytes(&marker)?,
        )?;
        let connection = open_connection(&canonical_root)?;
        initialize_schema(&connection, &marker, config.clock.now())?;
        drop(connection);

        let coordinator = coordinator_for(&canonical_root);
        coordinator.active_handles.fetch_add(1, Ordering::SeqCst);
        let admin = StoreAdmin {
            root: canonical_root.clone(),
            allowed_parent,
            store_uuid: marker.store_uuid,
            deletion_nonce: marker.deletion_nonce,
            coordinator: Arc::clone(&coordinator),
        };
        let store = Self {
            root: canonical_root,
            marker,
            coordinator,
            clock: config.clock,
            max_object_bytes: config.max_object_bytes,
            purge_registry_targets: config.purge_registry_targets,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "semantic-test-failpoints")]
            pause_after_commit: Arc::default(),
        };
        Ok((store, admin))
    }

    pub fn open(root: impl AsRef<Path>, config: SemanticConfig) -> Result<Self> {
        Self::open_with_gate_behavior(root, config, GateBehavior::Serve)
    }

    /// Opens a store for the explicit purpose of running a schema upgrade
    /// (Task F3.3). The schema-version gate that normally refuses to serve
    /// an older store (refuse-to-serve-until-upgraded — see
    /// [`Self::open`] + `validate_database_identity`) is RELAXED here: a
    /// store whose `schema_version` has a known migration path to
    /// `CURRENT_DISK_SCHEMA_VERSION` (today: 2 → 3) opens successfully so
    /// `plan_schema_upgrade` + `execute_schema_upgrade` can run. A genuinely
    /// unsupported version still fails closed.
    ///
    /// This is the entry point the `llm-wiki recovery upgrade` CLI subcommand
    /// uses; the running SERVER (`Commands::Serve`) keeps using [`Self::open`]
    /// so a v2 store cannot accidentally serve traffic under a v3 binary.
    pub fn open_for_upgrade(root: impl AsRef<Path>, config: SemanticConfig) -> Result<Self> {
        Self::open_with_gate_behavior(root, config, GateBehavior::UpgradeOnly)
    }

    /// Shared body of [`Self::open`] and [`Self::open_for_upgrade`]. The
    /// `gate` parameter picks which schema-version branch the
    /// `validate_database_identity` check tolerates:
    ///
    ///   * `Serve` — the running-server path. A marker at any version other
    ///     than `CURRENT_DISK_SCHEMA_VERSION` fails closed (refuse-to-serve
    ///     for known paths, hard-fail for unsupported).
    ///   * `UpgradeOnly` — the upgrade-CLI path. A marker at a version with
    ///     a known migration path is allowed through (so the upgrade can
    ///     run); genuinely unsupported versions still fail closed.
    fn open_with_gate_behavior(
        root: impl AsRef<Path>,
        config: SemanticConfig,
        gate: GateBehavior,
    ) -> Result<Self> {
        let (_, allowed_parent) = validate_requested_root(root.as_ref(), &config)?;
        validate_object_limit(&config)?;
        reject_link_or_reparse(root.as_ref())?;
        let canonical_root = root.as_ref().canonicalize().map_err(io_error)?;
        if canonical_root.parent() != Some(allowed_parent.as_path()) {
            return Err(SemanticError::InvalidRoot(
                "root escaped allowed parent".to_owned(),
            ));
        }
        let coordinator = coordinator_for(&canonical_root);
        let _maintenance = coordinator.maintenance.read();
        let _writer = coordinator.writer.lock();
        let marker = read_marker(&canonical_root)?;
        if marker.allowed_parent != allowed_parent.to_string_lossy() {
            return Err(SemanticError::MarkerMismatch);
        }
        let connection = open_connection(&canonical_root)?;
        validate_database_identity(&connection, &marker, gate)?;
        validate_ledger(&connection)?;
        // Restore/open-time fail-closed check (ADR Decision 7): a stale,
        // unavailable, or fork-diverged purge registry must seal the store
        // before any plaintext is exposed, even though this connection's own
        // read of purge_denied_ids is already consistent -- the risk here is
        // a DIFFERENT store replica having denied IDs this copy never heard
        // about (e.g. this root is a backup restore), not a same-process race.
        let sealed = evaluate_registry_seal(&connection, &config.purge_registry_targets)?;
        set_registry_sealed(&connection, sealed)?;
        drop(connection);
        coordinator.active_handles.fetch_add(1, Ordering::SeqCst);
        drop(_writer);
        drop(_maintenance);
        Ok(Self {
            root: canonical_root,
            marker,
            coordinator,
            clock: config.clock,
            max_object_bytes: config.max_object_bytes,
            purge_registry_targets: config.purge_registry_targets,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "semantic-test-failpoints")]
            pause_after_commit: Arc::default(),
        })
    }

    pub fn trusted_context(&self) -> TrustedContext {
        TrustedContext {
            store_uuid: self.marker.store_uuid,
            owner_id: self.marker.owner_id,
            actor_id: self.marker.actor_id,
            client_id: self.marker.client_id,
        }
    }

    /// Mints a context for a named client, generating and persisting a
    /// server-assigned UUIDv7 on first registration. The same label always
    /// resolves to the same client identity, so a client's idempotency scope
    /// survives process restarts. Owner and actor stay marker-bound.
    pub fn register_client(&self, label: &str) -> Result<TrustedContext> {
        self.register_client_scoped(label, DEFAULT_CLIENT_CAPABILITIES)
    }

    /// Registers (or resolves) a client like `register_client`, but grants
    /// only the given capabilities on first registration -- e.g. an empty
    /// list creates a pure propose-only worker identity (ADR Decision 8).
    /// Re-registering an existing label never changes its prior grants,
    /// even if a broader or narrower list is passed the second time.
    pub fn register_client_scoped(
        &self,
        label: &str,
        capabilities: &[&str],
    ) -> Result<TrustedContext> {
        validate_client_label(label)?;
        for capability in capabilities {
            if !VALID_CLIENT_CAPABILITIES.contains(capability) {
                return Err(SemanticError::InvalidClaim(format!(
                    "unknown capability: {capability}"
                )));
            }
        }
        let _maintenance = self.coordinator.maintenance.read();
        let _writer = self.coordinator.writer.lock();
        let mut connection = open_connection(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let active = ActiveTransaction::new(&self.coordinator);
        let existing: Option<String> = transaction
            .query_row(
                "SELECT client_id FROM clients WHERE label=?1",
                [label],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error)?;
        let client_id = match existing {
            Some(value) => Uuid::parse_str(&value).map_err(|_| {
                SemanticError::CorruptLedger("registered client has invalid UUID".to_owned())
            })?,
            None => {
                let client_id = Uuid::now_v7();
                transaction
                    .execute(
                        "INSERT INTO clients(client_id,label,created_at) VALUES (?1,?2,?3)",
                        params![client_id.to_string(), label, self.clock.now().to_rfc3339()],
                    )
                    .map_err(database_error)?;
                for capability in capabilities {
                    transaction
                        .execute(
                            "INSERT INTO client_capabilities(client_id,capability,granted_at) VALUES (?1,?2,?3)",
                            params![client_id.to_string(), capability, self.clock.now().to_rfc3339()],
                        )
                        .map_err(database_error)?;
                }
                client_id
            }
        };
        transaction.commit().map_err(database_error)?;
        drop(active);
        Ok(TrustedContext {
            store_uuid: self.marker.store_uuid,
            owner_id: self.marker.owner_id,
            actor_id: self.marker.actor_id,
            client_id,
        })
    }

    /// Snapshots the store into a fresh sibling root under the same allowed
    /// parent. Holds the maintenance write lock so no mutation, projection,
    /// or recovery runs while the database (via `VACUUM INTO`), marker,
    /// objects, and projection snapshot are copied.
    pub fn backup_consistent(&self, target_root: impl AsRef<Path>) -> Result<()> {
        let target = target_root.as_ref();
        if target.exists() {
            return Err(SemanticError::InvalidRoot(
                "backup target already exists".to_owned(),
            ));
        }
        let parent = target
            .parent()
            .ok_or_else(|| SemanticError::InvalidRoot("backup target has no parent".to_owned()))?;
        let canonical_parent = parent.canonicalize().map_err(|_| {
            SemanticError::InvalidRoot("backup target parent is not accessible".to_owned())
        })?;
        if canonical_parent.to_string_lossy() != self.marker.allowed_parent {
            return Err(SemanticError::InvalidRoot(
                "backup target escaped allowed parent".to_owned(),
            ));
        }

        let _maintenance = self.coordinator.maintenance.write();
        if self.coordinator.active_transactions.load(Ordering::SeqCst) != 0 {
            return Err(SemanticError::ActiveHandles);
        }
        fs::create_dir(target).map_err(io_error)?;
        fs::create_dir(target.join("staging")).map_err(io_error)?;
        fs::create_dir(target.join("objects")).map_err(io_error)?;

        let connection = open_connection(&self.root)?;
        let target_database = target.join(DATABASE_FILE);
        connection
            .execute(
                "VACUUM INTO ?1",
                [target_database.to_string_lossy().as_ref()],
            )
            .map_err(database_error)?;
        drop(connection);
        let copy = Connection::open(&target_database).map_err(database_error)?;
        copy.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(database_error)?;
        drop(copy);

        fs::copy(self.root.join(MARKER_FILE), target.join(MARKER_FILE)).map_err(io_error)?;
        let projection = self.root.join(PROJECTION_FILE);
        if projection.is_file() {
            fs::copy(&projection, target.join(PROJECTION_FILE)).map_err(io_error)?;
        }
        for shard in fs::read_dir(self.root.join("objects")).map_err(io_error)? {
            let shard = shard.map_err(io_error)?.path();
            if !shard.is_dir() {
                continue;
            }
            let shard_name = shard.file_name().ok_or_else(|| {
                SemanticError::CorruptLedger("object shard has no name".to_owned())
            })?;
            let target_shard = target.join("objects").join(shard_name);
            fs::create_dir_all(&target_shard).map_err(io_error)?;
            for entry in fs::read_dir(&shard).map_err(io_error)? {
                let path = entry.map_err(io_error)?.path();
                if !path.is_file() {
                    continue;
                }
                let file_name = path.file_name().ok_or_else(|| {
                    SemanticError::CorruptLedger("object file has no name".to_owned())
                })?;
                fs::copy(&path, target_shard.join(file_name)).map_err(io_error)?;
            }
        }
        // Recorded in THIS store's own database (not the backup copy) so a
        // future hard-purge saga's retention_pending step knows which
        // backups it made and may still be able to decrypt the target.
        let record_connection = open_connection(&self.root)?;
        record_connection
            .execute(
                "INSERT INTO purge_backup_sets(backup_path,created_at,invalidated_at) VALUES (?1,?2,NULL)",
                params![target.to_string_lossy(), self.clock.now().to_rfc3339()],
            )
            .map_err(database_error)?;
        Ok(())
    }

    /// Path to the encrypted-backup key file (Task F3.1).
    ///
    /// `<root>/backup.key` — 32 raw bytes that key an `Aes256Gcm` cipher.
    /// This is the operator-managed backup encryption key, distinct from the
    /// object-at-rest epoch keys: object crypto-shred rotates per-epoch DEKs
    /// wrapped under those; backup encrypts the *whole store snapshot* (db +
    /// objects + marker + projection) under this single long-lived key so a
    /// restored backup can be unlocked with one secret.
    ///
    /// The key is intentionally stored OUTSIDE any encrypted backup (a
    /// backup encrypted under a key it also contains would be unrecoverable
    /// — chicken-and-egg). Operators MUST back this file up separately
    /// (off-host, access-controlled). If lost, all backups taken under it
    /// are unrecoverable.
    pub fn backup_key_path(root: impl AsRef<Path>) -> PathBuf {
        root.as_ref().join(BACKUP_KEY_FILE)
    }

    /// Loads — or creates on first call — the backup encryption key (Task F3.1).
    ///
    /// On Unix the file is created atomically with mode 0600 (only the owning
    /// uid can read it) via `OpenOptionsExt::mode(0o600)`: the file is NEVER
    /// observable on disk in a world-readable mode, even transiently — a
    /// failure to set the mode fails the create (Task F3.2 carry fix 1; the
    /// prior `restrict_file_permissions` `warn!` + return Ok path could leave
    /// the key world-readable if chmod failed after create).
    ///
    /// On Windows there is no equivalent std API so the file inherits the
    /// user-profile default ACL, which is also user-only in practice, but
    /// operators running under a shared account should add an explicit ACL
    /// (`icacls backup.key /inheritance:r /grant:r "%USERNAME%:R"`).
    ///
    /// A malformed existing file (wrong length) is rejected — never silently
    /// rewritten — so a corrupted `backup.key` is loud rather than turning
    /// prior backups into unrecoverable ciphertext under a fresh key.
    pub fn load_or_create_backup_key(root: &Path) -> Result<Key<Aes256Gcm>> {
        let key_path = Self::backup_key_path(root);
        if key_path.is_file() {
            let bytes = fs::read(&key_path).map_err(io_error)?;
            if bytes.len() != AES_256_KEY_LEN {
                return Err(SemanticError::CorruptLedger(format!(
                    "backup.key is {} bytes (expected {}); refusing to overwrite a \
                     potentially-corrupted key that may unlock existing backups",
                    bytes.len(),
                    AES_256_KEY_LEN
                )));
            }
            let key = Key::<Aes256Gcm>::from_slice(&bytes);
            return Ok(*key);
        }
        let bytes = Aes256Gcm::generate_key(&mut OsRng);
        write_new_secret_file(&key_path, bytes.as_slice())?;
        Ok(bytes)
    }

    /// Produces an AES-GCM-encrypted full-store snapshot (Task F3.1).
    ///
    /// Stages a plaintext [`Self::backup_consistent`] snapshot in a temporary
    /// sibling directory, then encrypts every layer (sqlite db, marker,
    /// projection, every object shard blob) under the store's
    /// [`Self::load_or_create_backup_key`] into `<target_root>/<name>.enc`
    /// files, writes a plaintext `manifest.json` (composite checksum + layer
    /// list + created-at) into the target, and removes the staging dir. The
    /// returned [`BackupReport`] carries `encrypted: true` and the live
    /// store's `composite_checksum` so a clean-host restore drill can verify
    /// the decrypted snapshot matches the source.
    ///
    /// Each `.enc` file is `nonce || ciphertext` with a fresh random 12-byte
    /// nonce per file (negligible collision risk at backup cadence — well
    /// below the AES-GCM 2^32 message bound). The cipher is the same
    /// `Aes256Gcm` used for object-at-rest crypto-shred; the KEY is a
    /// separate backup-only key (`<root>/backup.key`) so destroying an
    /// object's epoch key never also destroys a backup.
    ///
    /// Like `backup_consistent`, this requires the target directory to NOT
    /// pre-exist and to live under the store's `allowed_parent` (the
    /// backup-consistent escape-prevention guard). The plaintext staging dir
    /// is wiped on success AND on the error paths below.
    pub fn backup_encrypted(
        &self,
        target_root: impl AsRef<Path>,
    ) -> Result<crate::recovery::BackupReport> {
        let target = target_root.as_ref();
        if target.exists() {
            return Err(SemanticError::InvalidRoot(
                "encrypted backup target already exists".to_owned(),
            ));
        }
        let parent = target.parent().ok_or_else(|| {
            SemanticError::InvalidRoot("encrypted backup target has no parent".to_owned())
        })?;
        let canonical_parent = parent.canonicalize().map_err(|_| {
            SemanticError::InvalidRoot(
                "encrypted backup target parent is not accessible".to_owned(),
            )
        })?;
        if canonical_parent.to_string_lossy() != self.marker.allowed_parent {
            return Err(SemanticError::InvalidRoot(
                "encrypted backup target escaped allowed parent".to_owned(),
            ));
        }

        // 1. Stage a plaintext snapshot in a sibling temp dir, then VACUUM/copy
        //    into it via backup_consistent. The staging dir MUST be a sibling
        //    of the target so backup_consistent's `canonical_parent` guard
        //    accepts it (it has to live under self.marker.allowed_parent).
        let staging_name = format!(
            ".{}-staging",
            target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("enc-backup")
        );
        let staging = parent.join(&staging_name);
        if staging.exists() {
            // A prior crashed run left a staging dir; remove it so this run
            // starts clean. Never remove the target — that is operator data.
            let _ = fs::remove_dir_all(&staging);
        }

        // Drop-guard: wipe the staging dir on scope exit regardless of how we
        // leave it (normal return, `?` error propagation, OR panic unwind).
        // The staging dir contains the plaintext snapshot — defense-in-depth
        // against a rusqlite panic or stray `.unwrap()` mid-backup leaking
        // decrypted bytes at `parent/.{target}-staging/`. `run_encrypted_backup_inner`
        // writes the ENCRYPTED target into `target`, never renames `staging`
        // into anything, so the guard is never disarmed here.
        let staging_guard = StagingDirGuard::new(staging.clone());
        let report = self.run_encrypted_backup_inner(&staging, target);
        drop(staging_guard);
        report
    }

    fn run_encrypted_backup_inner(
        &self,
        staging: &Path,
        target: &Path,
    ) -> Result<crate::recovery::BackupReport> {
        // 2. Plaintext snapshot via the existing VACUUM-into + objects-copy
        //    path. This also inserts a row into THIS store's purge_backup_sets.
        self.backup_consistent(staging)?;

        // 3. Load (or first-create) the backup key. Loaded here, AFTER
        //    backup_consistent succeeded, so a key-create failure never
        //    leaves a half-written plaintext snapshot lying around in a path
        //    that the staging-dir cleanup below would not reach (the key
        //    file lives inside self.root, not under staging).
        let key = Self::load_or_create_backup_key(&self.root)?;
        let cipher = Aes256Gcm::new(&key);

        fs::create_dir(target).map_err(io_error)?;

        // 4. Encrypt every layer. Each .enc file is nonce||ciphertext.
        let mut objects_count: u64 = 0;
        // Per-blob plaintext digests (F3.2 review Fix 2). Keyed by the
        // object path relative to the `objects/` root (e.g.
        // `"ab/cd/abcd1234..."`); valued by `sha256(plaintext)` lowercase
        // hex. Filled alongside encryption below; carried by the manifest
        // so restore can fail-closed if a `.enc` blob is substituted with a
        // legitimately-encrypted blob from a DIFFERENT object under the same
        // key (AES-GCM authenticates the ciphertext but not the binding to
        // THIS specific backup).
        let mut object_digests: HashMap<String, String> = HashMap::new();
        encrypt_file_into(
            &cipher,
            &staging.join(DATABASE_FILE),
            &target.join(format!("{DATABASE_FILE}.enc")),
        )?;
        encrypt_file_into(
            &cipher,
            &staging.join(MARKER_FILE),
            &target.join(format!("{MARKER_FILE}.enc")),
        )?;
        if staging.join(PROJECTION_FILE).is_file() {
            encrypt_file_into(
                &cipher,
                &staging.join(PROJECTION_FILE),
                &target.join(format!("{PROJECTION_FILE}.enc")),
            )?;
        }
        // Object shard tree: mirror the shard/<file> layout, each blob .enc'd.
        let staging_objects = staging.join("objects");
        if staging_objects.is_dir() {
            for shard in fs::read_dir(&staging_objects).map_err(io_error)? {
                let shard = shard.map_err(io_error)?.path();
                if !shard.is_dir() {
                    continue;
                }
                let shard_name = shard.file_name().ok_or_else(|| {
                    SemanticError::CorruptLedger("object shard has no name".to_owned())
                })?;
                let shard_name_str = shard_name.to_string_lossy().into_owned();
                let target_shard = target.join("objects").join(shard_name);
                fs::create_dir_all(&target_shard).map_err(io_error)?;
                for entry in fs::read_dir(&shard).map_err(io_error)? {
                    let path = entry.map_err(io_error)?.path();
                    if !path.is_file() {
                        continue;
                    }
                    let file_name = path.file_name().ok_or_else(|| {
                        SemanticError::CorruptLedger("object file has no name".to_owned())
                    })?;
                    // Read the plaintext once, hash it for the manifest's
                    // per-blob digest map, then hand it to the encryptor
                    // (which re-reads — kept separate so the encrypt path
                    // stays unchanged and the digest is over exactly the
                    // bytes that hit disk).
                    let plaintext = fs::read(&path).map_err(io_error)?;
                    let mut hasher = Sha256::new();
                    hasher.update(&plaintext);
                    let digest_hex = hex::encode(hasher.finalize());
                    // Object blob files are content-addressed hex digests
                    // with no extension; the encrypted sibling is just
                    // "<digest>.enc" so the original name stays readable.
                    let file_name_str = file_name.to_string_lossy().into_owned();
                    let enc_name = format!("{}.enc", file_name_str);
                    encrypt_file_into(&cipher, &path, &target_shard.join(enc_name))?;
                    object_digests.insert(format!("{shard_name_str}/{file_name_str}"), digest_hex);
                    objects_count = objects_count.saturating_add(1);
                }
            }
        }

        // 5. Live composite checksum — for restore-drill verification.
        let checksum = self.composite_checksum()?;

        // 6. Count ledger rows in the snapshot (run against the freshly
        //    snapshotted db so the count is exactly what a restore would
        //    see, not a racing live value). The same connection also yields
        //    the snapshot's PurgeRegistry head epoch (recorded in the manifest
        //    so a restore drill can fail-closed if the decrypted db's registry
        //    diverged — Task F3.2).
        let snapshot_db = staging.join(DATABASE_FILE);
        let snapshot_connection = Connection::open(&snapshot_db).map_err(database_error)?;
        let ledger_events_backed_up = count_ledger_events(&snapshot_db)?;
        let purge_epoch = snapshot_registry_epoch(&snapshot_connection)?;

        // 7. Manifest — plaintext JSON so operators can inspect the backup
        //    without decrypting (layer list + checksum + created_at). The
        //    cipher key is NOT in here (chicken-and-egg); the manifest only
        //    records what to decrypt and how to verify it.
        let manifest = BackupManifest {
            version: BACKUP_MANIFEST_VERSION,
            encrypted: true,
            cipher: BACKUP_CIPHER_ALG.to_owned(),
            composite_checksum: checksum.clone(),
            created_at: self.clock.now().to_rfc3339(),
            objects_count,
            ledger_events_count: ledger_events_backed_up,
            purge_epoch,
            objects: object_digests,
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(serialization_error)?;
        write_new_file(&target.join(BACKUP_MANIFEST_FILE), &manifest_bytes)?;

        // 8. Report. config_snapshot=true (marker always present), git=0
        //    until a git-history layer is wired (Phase F follow-up).
        Ok(crate::recovery::BackupReport::for_encrypted(
            objects_count,
            ledger_events_backed_up,
            checksum,
        ))
    }

    /// Restores an encrypted backup into a fresh target directory (Task F3.2).
    ///
    /// Reverse of [`Self::backup_encrypted`]: reads `manifest.json` from
    /// `backup_root`, decrypts every `.enc` layer under the supplied `key`,
    /// materializes the snapshot at `target_root` (which must NOT pre-exist
    /// and must live under this store's `allowed_parent`, matching the
    /// backup-side escape-prevention guard), opens the restored store, and
    /// verifies BOTH:
    ///
    /// 1. **Composite checksum match** — the restored store's
    ///    `composite_checksum()` must equal the manifest's recorded
    ///    `composite_checksum`. A mismatch means the snapshot was corrupted
    ///    or tampered with after backup.
    /// 2. **PurgeRegistry epoch match** — the restored store's
    ///    `local_registry_head()` epoch must equal the manifest's recorded
    ///    `purge_epoch`. A mismatch means the registry diverged between
    ///    backup and restore (e.g. a denial landed in the source store after
    ///    the snapshot, but the snapshot db is what a restore would actually
    ///    serve), which fails the drill closed per §9.4.
    ///
    /// On success returns a [`RestoreReceipt`] carrying the recomputed
    /// checksum + the layer names actually decrypted + the registry-sync
    /// verdict. The cipher `key` is taken as a parameter (read by the caller
    /// from the operator-managed `<root>/backup.key`) so this routine stays
    /// testable without depending on the live store's own key file.
    ///
    /// `target_root` is treated as operator data — it is NEVER removed by
    /// this routine, on success or failure (the staging dir, by contrast, is
    /// always wiped).
    pub fn restore_from_backup(
        &self,
        backup_root: impl AsRef<Path>,
        target_root: impl AsRef<Path>,
        key: &Key<Aes256Gcm>,
    ) -> Result<crate::recovery::RestoreReceipt> {
        let backup = backup_root.as_ref();
        let target = target_root.as_ref();

        // Escape guard: target must be a fresh dir under allowed_parent.
        if target.exists() {
            return Err(SemanticError::InvalidRoot(
                "restore target already exists".to_owned(),
            ));
        }
        let parent = target
            .parent()
            .ok_or_else(|| SemanticError::InvalidRoot("restore target has no parent".to_owned()))?;
        let canonical_parent = parent.canonicalize().map_err(|_| {
            SemanticError::InvalidRoot("restore target parent is not accessible".to_owned())
        })?;
        if canonical_parent.to_string_lossy() != self.marker.allowed_parent {
            return Err(SemanticError::InvalidRoot(
                "restore target escaped allowed parent".to_owned(),
            ));
        }

        // 1. Load + validate the manifest.
        let manifest_path = backup.join(BACKUP_MANIFEST_FILE);
        let manifest_bytes = fs::read(&manifest_path).map_err(io_error).map_err(|_| {
            SemanticError::CorruptLedger(format!(
                "backup manifest missing at {}",
                manifest_path.display()
            ))
        })?;
        let manifest: BackupManifest =
            serde_json::from_slice(&manifest_bytes).map_err(serialization_error)?;
        if !manifest.encrypted {
            return Err(SemanticError::CorruptLedger(
                "manifest declares an unencrypted backup; restore_from_backup only handles \
                 AES-256-GCM encrypted snapshots"
                    .to_owned(),
            ));
        }
        if !manifest.cipher.eq_ignore_ascii_case(BACKUP_CIPHER_ALG) {
            return Err(SemanticError::CorruptLedger(format!(
                "manifest cipher `{}` is not supported (expected `{BACKUP_CIPHER_ALG}`)",
                manifest.cipher
            )));
        }

        let cipher = Aes256Gcm::new(key);

        // 2. Stage decrypted layers in a sibling temp dir (mirrors
        //    backup_encrypted's staging layout, reversed). The staging dir
        //    is ALWAYS wiped, success or failure — the only operator data
        //    is `target`, never `staging`.
        let staging_name = format!(
            ".{}-restore-staging",
            target
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("restore")
        );
        let staging = parent.join(&staging_name);
        if staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }

        // Drop-guard: wipe the staging dir on scope exit regardless of how we
        // leave it (normal return, `?` error propagation, OR panic unwind).
        // The staging dir holds the DECRYPTED layers — defense-in-depth
        // against a panic mid-`run_restore_inner` leaking plaintext at
        // `parent/.{target}-restore-staging/`. On the success path
        // `run_restore_inner` renames `staging` into `target` atomically; the
        // guard is then disarmed (staging no longer exists at the path, and
        // `target` is operator data we must NOT wipe).
        let staging_guard = StagingDirGuard::new(staging.clone());
        let result = self.run_restore_inner(backup, target, &staging, &cipher, &manifest);
        match &result {
            Ok(_) => staging_guard.disarm(),
            Err(_) => drop(staging_guard),
        }
        result
    }

    fn run_restore_inner(
        &self,
        backup: &Path,
        target: &Path,
        staging: &Path,
        cipher: &Aes256Gcm,
        manifest: &BackupManifest,
    ) -> Result<crate::recovery::RestoreReceipt> {
        fs::create_dir(staging).map_err(io_error)?;
        fs::create_dir(staging.join("objects")).map_err(io_error)?;

        let mut layers_restored: Vec<String> = Vec::new();

        // DB layer (always present).
        let db_bytes = decrypt_backup_layer(cipher, &backup.join(format!("{DATABASE_FILE}.enc")))?;
        write_new_file(&staging.join(DATABASE_FILE), &db_bytes)?;
        layers_restored.push("db".to_owned());

        // Marker layer (always present).
        let marker_bytes =
            decrypt_backup_layer(cipher, &backup.join(format!("{MARKER_FILE}.enc")))?;
        write_new_file(&staging.join(MARKER_FILE), &marker_bytes)?;
        layers_restored.push("marker".to_owned());

        // Projection layer (optional).
        let projection_enc = backup.join(format!("{PROJECTION_FILE}.enc"));
        if projection_enc.is_file() {
            let projection_bytes = decrypt_backup_layer(cipher, &projection_enc)?;
            write_new_file(&staging.join(PROJECTION_FILE), &projection_bytes)?;
            layers_restored.push("projection".to_owned());
        }

        // Object shard tree (optional + per-blob).
        let backup_objects = backup.join("objects");
        if backup_objects.is_dir() {
            for shard in fs::read_dir(&backup_objects).map_err(io_error)? {
                let shard = shard.map_err(io_error)?.path();
                if !shard.is_dir() {
                    continue;
                }
                let shard_name = shard.file_name().ok_or_else(|| {
                    SemanticError::CorruptLedger("object shard has no name".to_owned())
                })?;
                let shard_name_str = shard_name.to_string_lossy().into_owned();
                let staging_shard = staging.join("objects").join(shard_name);
                fs::create_dir_all(&staging_shard).map_err(io_error)?;
                for entry in fs::read_dir(&shard).map_err(io_error)? {
                    let path = entry.map_err(io_error)?.path();
                    if !path.is_file() {
                        continue;
                    }
                    let file_name = path.file_name().ok_or_else(|| {
                        SemanticError::CorruptLedger("object file has no name".to_owned())
                    })?;
                    let plain_name = file_name.to_string_lossy();
                    // Encrypted blob files end in ".enc"; strip the suffix to
                    // recover the content-addressed digest name.
                    let plain_name = plain_name
                        .strip_suffix(".enc")
                        .unwrap_or(&plain_name)
                        .to_owned();
                    let bytes = decrypt_backup_layer(cipher, &path)?;
                    // Per-blob digest verification (F3.2 review Fix 2).
                    // AES-GCM authenticates the ciphertext but not its
                    // binding to THIS specific backup — a substituted blob
                    // from a different object under the same key would
                    // otherwise decrypt cleanly. The manifest's `objects`
                    // map binds each `<shard>/<digest>` path to the original
                    // plaintext's sha256, so a substitution fails closed
                    // here. F3.1 manifests have an empty `objects` map
                    // (no digests recorded) → log + skip verification for
                    // back-compat.
                    let relative_key = format!("{shard_name_str}/{plain_name}");
                    if let Some(expected_digest) = manifest.objects.get(&relative_key) {
                        let mut hasher = Sha256::new();
                        hasher.update(&bytes);
                        let actual_digest = hex::encode(hasher.finalize());
                        if &actual_digest != expected_digest {
                            return Err(SemanticError::CorruptLedger(format!(
                                "object blob digest mismatch at {relative_key}: \
                                 manifest={expected_digest} restored={actual_digest} \
                                 (blob may have been substituted with a legitimately-encrypted \
                                 blob from a different object under the same key)"
                            )));
                        }
                    } else if !manifest.objects.is_empty() {
                        // The map is non-empty but this blob is unlisted —
                        // treat as tampering (a real backup lists every blob
                        // it encrypts; a missing entry means the on-disk tree
                        // has more blobs than the manifest, which can only
                        // happen via post-hoc injection).
                        return Err(SemanticError::CorruptLedger(format!(
                            "object blob at {relative_key} is not listed in the manifest's \
                             per-blob digest map (manifest lists {} blobs; this blob is extra)",
                            manifest.objects.len()
                        )));
                    } else {
                        tracing::warn!(
                            target: "semantic::restore",
                            relative_path = %relative_key,
                            "restoring F3.1-era backup (no per-blob digests in manifest); \
                             skipping blob digest verification for this path"
                        );
                    }
                    write_new_file(&staging_shard.join(&plain_name), &bytes)?;
                }
            }
            layers_restored.push("objects".to_owned());
        }

        // 3. Materialize the restored store at `target`. The staging layout
        //    matches what SemanticStore::open expects (db + marker +
        //    projection + objects/<shard>/<digest>). We rename staging INTO
        //    target so the materialization is atomic from the filesystem's
        //    perspective — `target` either does not exist (this routine is
        //    still running) or holds a complete snapshot.
        fs::rename(staging, target).map_err(|error| {
            SemanticError::Io(format!(
                "failed to materialize restored store at {}: {error}",
                target.display()
            ))
        })?;

        // 4. Open the restored store to verify integrity. The store was
        //    created via VACUUM INTO, so it has a consistent schema,
        //    marker-bound identity, and a populated PurgeRegistry — open()
        //    runs validate_database_identity + validate_ledger +
        //    evaluate_registry_seal over it.
        let config = SemanticConfig::enabled_for(Path::new(&self.marker.allowed_parent));
        let restored = Self::open(target, config)?;

        // 5. Composite checksum match — fail-closed if the snapshot was
        //    corrupted or tampered with after backup.
        let recomputed = restored.composite_checksum()?;
        if recomputed != manifest.composite_checksum {
            return Err(SemanticError::CorruptLedger(format!(
                "restore composite checksum mismatch: manifest={} restored={recomputed}",
                manifest.composite_checksum
            )));
        }

        // 6. PurgeRegistry epoch match — fail-closed if the registry diverged
        //    between backup and restore (the snapshot db's registry head
        //    MUST equal the manifest's recorded purge_epoch). Note that a
        //    mismatch here cannot be fixed by re-running sync_purge_registry
        //    on the restored store: the divergence is between the snapshot
        //    the operator is trying to restore and the live registry state at
        //    backup time, which is exactly the §9.4 "registry unavailable /
        //    stale → fail closed" signal.
        let restored_epoch = restored.registry_epoch().unwrap_or(0);
        let purge_registry_synced = restored_epoch == manifest.purge_epoch;

        Ok(crate::recovery::RestoreReceipt {
            state: "completed".to_owned(),
            composite_checksum: recomputed,
            layers_restored,
            purge_registry_synced,
            restored_at: self.clock.now(),
        })
    }

    /// Runs a clean-host restore drill against `backup_root` (Task F3.2).
    ///
    /// Drills into a fresh temporary target (a sibling of this store's root,
    /// so the result is restorable under the same `allowed_parent`),
    /// decrypts + verifies every layer, then writes
    /// `<self.root>/restore-drill.json` recording the outcome — the file
    /// that [`Self::backup_health`] reads to report
    /// `last_restore_drill_ok`. Always writes the outcome file, success OR
    /// failure, so an operator can inspect the last drill's verdict even
    /// when the drill itself errored (Phase E3.1 left this read of an
    /// unwritten file as a stub; this method closes the loop).
    ///
    /// Returns a [`RecoveryDrillResult`] whose `passed()` is `true` only when
    /// BOTH the registry synced AND the composite checksum matched (the
    /// §9.4 fail-closed contract). On any decryption / verification error
    /// the result is reported with both flags false and the error string is
    /// included in the outcome file.
    pub fn run_restore_drill(
        &self,
        backup_root: impl AsRef<Path>,
        key: &Key<Aes256Gcm>,
    ) -> Result<crate::recovery::RecoveryDrillResult> {
        let backup = backup_root.as_ref();
        let parent = Path::new(&self.marker.allowed_parent);
        // Drill target: a sibling temp dir. The name is unique per drill so
        // repeated runs do not collide; it is wiped at the end of this call.
        let drill_target = parent.join(format!(".drill-{}", Uuid::now_v7().simple()));
        if drill_target.exists() {
            let _ = fs::remove_dir_all(&drill_target);
        }

        let outcome = self.restore_from_backup(backup, &drill_target, key);
        // Always clean up the drill target — it is throwaway verification
        // data, not a real restore.
        if drill_target.exists() {
            let _ = fs::remove_dir_all(&drill_target);
        }

        let now = self.clock.now();
        match outcome {
            Ok(receipt) => {
                // restore_from_backup already verifies the recomputed checksum
                // equals the manifest's; reaching the Ok path means the
                // composite checksum matched. The drill's checksum_matches
                // flag is therefore always true on the success branch. The
                // overall `last_ok` reflects `passed()` — a registry-epoch
                // mismatch makes the drill fail closed even though the
                // underlying restore_from_backup succeeded.
                let result = crate::recovery::RecoveryDrillResult {
                    objects_restored: receipt
                        .layers_restored
                        .iter()
                        .filter(|name| name.as_str() == "objects")
                        .count() as u64,
                    ledger_events_restored: 0,
                    purge_registry_synced: receipt.purge_registry_synced,
                    composite_checksum_matches: true,
                };
                self.write_restore_drill_outcome(
                    result.passed(),
                    now,
                    &receipt.composite_checksum,
                    receipt.purge_registry_synced,
                    receipt.layers_restored,
                    None,
                )?;
                Ok(result)
            }
            Err(error) => {
                let message = format!("{error}");
                self.write_restore_drill_outcome(
                    false,
                    now,
                    "",
                    false,
                    Vec::new(),
                    Some(&message),
                )?;
                Ok(crate::recovery::RecoveryDrillResult {
                    objects_restored: 0,
                    ledger_events_restored: 0,
                    purge_registry_synced: false,
                    composite_checksum_matches: false,
                })
            }
        }
    }

    /// Writes `<self.root>/restore-drill.json` (Task F3.2). The shape matches
    /// what [`Self::backup_health`] reads:
    /// `{last_ok, at, composite_checksum, purge_registry_synced,
    /// layers_restored, error?}`. The file is overwritten atomically via
    /// `write_new_file` semantics by first removing any prior copy — a drill
    /// rerun always reflects the latest outcome, never a stale success.
    fn write_restore_drill_outcome(
        &self,
        last_ok: bool,
        at: DateTime<Utc>,
        composite_checksum: &str,
        purge_registry_synced: bool,
        layers_restored: Vec<String>,
        error: Option<&str>,
    ) -> Result<()> {
        let body = serde_json::json!({
            "last_ok": last_ok,
            "at": at.to_rfc3339(),
            "composite_checksum": composite_checksum,
            "purge_registry_synced": purge_registry_synced,
            "layers_restored": layers_restored,
            "error": error,
        });
        let bytes = serde_json::to_vec_pretty(&body).map_err(serialization_error)?;
        let path = self.root.join("restore-drill.json");
        if path.exists() {
            // Replace the prior outcome — never leave a stale `last_ok: true`
            // visible after a failed drill.
            let _ = fs::remove_file(&path);
        }
        write_new_file(&path, &bytes)
    }

    // ── Phase F Task F3.3 — schema upgrade runner + RPO/RTO recorder ──────────
    //
    // The v2→v3 migration is a noop placeholder (every step is a reversible
    // noop), but the runner is real: it wraps the migration in a SQLite
    // transaction, atomically rewrites the marker file's schema_version, and
    // supports a reversible rollback. This proves the upgrade path end-to-end
    // so the next genuine DDL break is a one-step addition (a real UpgradeStep
    // action) rather than a new piece of infrastructure.

    /// Plans a schema upgrade from `from` to `to`. Supported paths today are
    /// `from=2, to=3` (Task F3.3 noop placeholder) and `from=3, to=4`
    /// (Entity Identity Reform — the step *description* now names the entity
    /// consolidation + UNIQUE/PRIMARY KEY constraint change; the step's
    /// forward/reverse *bodies* land in Tasks 3–5). Any other combination
    /// returns `Err(SemanticError::CorruptLedger(...))` with a message naming
    /// the unsupported pair — `plan_schema_upgrade` is the single source of
    /// truth for "which paths exist", so the schema-version gate in
    /// `validate_database_identity` mirrors it via `schema_upgrade_path_exists`.
    ///
    /// Each plan carries exactly one reversible step (the v2→v3 step is a noop
    /// placeholder; the v3→v4 step describes the consolidation), so
    /// `is_reversible()` is true and `execute_schema_upgrade` will accept it.
    pub fn plan_schema_upgrade(
        &self,
        from: u8,
        to: u8,
    ) -> Result<crate::recovery::SchemaUpgradePlan> {
        if !schema_upgrade_path_exists(from, to) {
            return Err(SemanticError::CorruptLedger(format!(
                "unsupported schema upgrade path: {from} → {to} (supported paths: 2→3, 3→4)"
            )));
        }
        // Sanity: the plan's `from` must equal the live marker version, since
        // execute_schema_upgrade writes `to_version` to disk and assumes the
        // marker was at `from_version` going in. A mismatch means the caller
        // is planning against the wrong store — fail closed rather than
        // produce a plan that would silently bump a different store's version.
        if from != self.marker.schema_version {
            return Err(SemanticError::CorruptLedger(format!(
                "plan from={from} does not match live marker schema_version={}; plan against \
                 the store's actual version",
                self.marker.schema_version
            )));
        }
        Ok(crate::recovery::SchemaUpgradePlan {
            steps: match (from, to) {
                (2, 3) => vec![crate::recovery::UpgradeStep {
                    description: "noop placeholder migration to prove upgrade path".to_owned(),
                    reversible: true,
                }],
                (3, 4) => vec![crate::recovery::UpgradeStep {
                    description: (
                        "Entity Identity Reform: consolidate fragmented entities onto one \
                         canonical_subject, then drop domain from the entities UNIQUE key \
                         and the entity_aliases PRIMARY KEY (Wikidata pattern)."
                    )
                        .to_owned(),
                    reversible: true,
                }],
                _ => unreachable!("schema_upgrade_path_exists gates this match"),
            },
            from_version: from,
            to_version: to,
        })
    }

    /// Executes a schema-upgrade plan transactionally. Safety contract:
    ///
    ///   * `plan.is_reversible()` MUST be true — refusing to execute a
    ///     non-reversible plan means an operator can always roll back from a
    ///     half-applied state. (Today every `plan_schema_upgrade` output is
    ///     reversible; this is a defense-in-depth check against future
    ///     contributors adding a non-reversible step.)
    ///   * The plan's `from_version` MUST equal the live marker's
    ///     `schema_version` — otherwise the marker file on disk is at a
    ///     different version than the plan assumes, and writing `to_version`
    ///     would silently corrupt the upgrade audit trail.
    ///
    /// On success: every step's forward action runs (noop for v2→v3), the
    /// `meta.schema_version` row is updated to `to_version`, AND the marker
    /// file on disk is rewritten atomically with the new `schema_version`.
    /// Both writes happen INSIDE a SQLite transaction so the DB half cannot
    /// commit without the step actions succeeding. The marker rewrite is
    /// best-effort atomic at the filesystem level (temp-file + rename): if
    /// it fails after the DB commit, the DB carries `to_version` but the
    /// marker file is stale — the operator reruns `recovery upgrade` (idempotent:
    /// plan_schema_upgrade will then return Err because from != live, but
    /// `validate_database_identity` already accepts `marker == CURRENT`).
    pub fn execute_schema_upgrade(&self, plan: &crate::recovery::SchemaUpgradePlan) -> Result<()> {
        if !plan.is_reversible() {
            return Err(SemanticError::CorruptLedger(
                "refusing to execute a non-reversible schema upgrade plan (rollback would be \
                 impossible)"
                    .to_owned(),
            ));
        }
        if plan.from_version != self.marker.schema_version {
            return Err(SemanticError::CorruptLedger(format!(
                "plan from_version={} does not match live marker schema_version={}; refusing to \
                 apply a plan against the wrong starting version",
                plan.from_version, self.marker.schema_version
            )));
        }
        if plan.from_version == plan.to_version {
            // No-op plan: nothing to do, but report success rather than
            // touching disk. (plan_schema_upgrade never produces this shape,
            // but an externally-constructed plan could.)
            return Ok(());
        }

        let _maintenance = self.coordinator.maintenance.write();
        if self.coordinator.active_transactions.load(Ordering::SeqCst) != 0 {
            return Err(SemanticError::ActiveHandles);
        }
        let mut connection = open_connection(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let active = ActiveTransaction::new(&self.coordinator);

        // Run each step's forward action. The v2→v3 step is a noop, but the
        // loop is here so a future genuine migration just adds an arm. Any
        // step failure rolls the transaction back via `?` (Drop on the
        // transaction issues ROLLBACK on a non-committed rusqlite txn).
        let now = self.clock.now();
        for (index, step) in plan.steps.iter().enumerate() {
            run_upgrade_step_forward(
                &transaction,
                plan.from_version,
                plan.to_version,
                index,
                step,
                now,
            )?;
        }

        // Update the DB-side schema_version row so validate_database_identity
        // (which reads marker.schema_version, NOT the meta row — see below)
        // and any future tooling that reads meta see the bumped version.
        transaction
            .execute(
                "UPDATE meta SET value=?1 WHERE key='schema_version'",
                params![plan.to_version.to_string()],
            )
            .map_err(database_error)?;

        transaction.commit().map_err(database_error)?;
        drop(active);
        drop(_maintenance);

        // Atomically rewrite the marker file with the new schema_version.
        // validate_database_identity on the NEXT open() reads
        // marker.schema_version, so this rewrite is what actually advances
        // the on-disk version an operator sees.
        self.rewrite_marker_with_version(plan.to_version)
    }

    /// Reverses a schema-upgrade plan: rewinds the DB meta row AND the marker
    /// file back to `plan.from_version`. Like `execute_schema_upgrade`, runs
    /// inside a SQLite transaction; any step failure rolls back.
    ///
    /// Use this only AFTER a successful `execute_schema_upgrade` (it is the
    /// "undo" half of the F3.3 round-trip rehearsal). Calling rollback
    /// without a prior execute leaves the store at from_version (a noop);
    /// calling it after a downgrade that was never applied is also a noop
    /// for the v2→v3 case because the step is reversible-via-noop.
    pub fn rollback_schema_upgrade(&self, plan: &crate::recovery::SchemaUpgradePlan) -> Result<()> {
        if !plan.is_reversible() {
            return Err(SemanticError::CorruptLedger(
                "refusing to roll back a non-reversible schema upgrade plan".to_owned(),
            ));
        }
        // The marker must currently be at to_version (the state execute
        // leaves it in). If it is not, the operator is calling rollback
        // against the wrong state — fail closed rather than rewind to an
        // unexpected from_version.
        if plan.to_version != self.marker.schema_version {
            return Err(SemanticError::CorruptLedger(format!(
                "plan to_version={} does not match live marker schema_version={}; rollback \
                 expects the store to be at the post-upgrade version",
                plan.to_version, self.marker.schema_version
            )));
        }
        if plan.from_version == plan.to_version {
            return Ok(());
        }

        let _maintenance = self.coordinator.maintenance.write();
        if self.coordinator.active_transactions.load(Ordering::SeqCst) != 0 {
            return Err(SemanticError::ActiveHandles);
        }
        let mut connection = open_connection(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let active = ActiveTransaction::new(&self.coordinator);

        // Reverse each step in REVERSE order. For the v2→v3 noop, the
        // forward and reverse actions are both noops, but the loop shape is
        // here so a future genuine migration's reverse actions land in the
        // right order.
        for (forward_index, step) in plan.steps.iter().enumerate().rev() {
            run_upgrade_step_reverse(
                &transaction,
                plan.from_version,
                plan.to_version,
                forward_index,
                step,
            )?;
        }

        transaction
            .execute(
                "UPDATE meta SET value=?1 WHERE key='schema_version'",
                params![plan.from_version.to_string()],
            )
            .map_err(database_error)?;

        transaction.commit().map_err(database_error)?;
        drop(active);
        drop(_maintenance);

        self.rewrite_marker_with_version(plan.from_version)
    }

    /// Atomically rewrites `<root>/store.marker.json` with `new_version`
    /// substituted for the current `schema_version` field. Every other
    /// marker field is preserved (store_uuid / owner_id / actor_id /
    /// client_id / deletion_nonce / allowed_parent — these ARE the store's
    /// persistent identity; only `schema_version` is upgradable).
    ///
    /// Atomicity: write to `store.marker.json.tmp` then rename over the
    /// existing marker. rename(2) on POSIX is atomic; on Windows it is
    /// atomic with respect to readers but a concurrent writer could
    /// interfere — the maintenance write-lock taken by the caller
    /// (`execute_schema_upgrade` / `rollback_schema_upgrade`) prevents that.
    fn rewrite_marker_with_version(&self, new_version: u8) -> Result<()> {
        let mut fresh = self.marker.clone();
        fresh.schema_version = new_version;
        let bytes = canonical_bytes(&fresh)?;
        let marker_path = self.root.join(MARKER_FILE);
        let tmp_path = self.root.join(format!("{MARKER_FILE}.tmp"));
        // write_new_file would refuse (create_new=true) if a `.tmp` from a
        // prior crashed run lingers, so wipe any stale tmp first.
        if tmp_path.exists() {
            let _ = fs::remove_file(&tmp_path);
        }
        write_new_file(&tmp_path, &bytes)?;
        fs::rename(&tmp_path, &marker_path).map_err(|error| {
            SemanticError::Io(format!(
                "failed to atomically rewrite marker at {}: {error}",
                marker_path.display()
            ))
        })
    }

    /// Records the operator's Recovery Point / Time Objectives to
    /// `<root>/rpo-rto.json`. The file is read back by [`Self::read_rpo_rto`]
    /// and is the source of truth for an SLO/SLA dashboard that wants to
    /// display "is the store meeting its RPO/RTO?" without re-prompting the
    /// operator every poll.
    ///
    /// `rpo_minutes` is the max acceptable data loss (the cadence at which
    /// `recovery backup` runs); `rto_minutes` is the max acceptable downtime
    /// (the target restore time). `met` records whether the LAST
    /// backup-or-restore measurement satisfied both. The file is overwritten
    /// atomically (temp + rename) so a partial write never leaves a
    /// half-recorded SLO visible to readers.
    pub fn record_rpo_rto(
        &self,
        rpo_minutes: u32,
        rto_minutes: u32,
        met: bool,
    ) -> Result<crate::recovery::RpoRto> {
        let record = crate::recovery::RpoRto {
            rpo_minutes,
            rto_minutes,
            last_met: met,
        };
        let body = serde_json::json!({
            "rpo_minutes": record.rpo_minutes,
            "rto_minutes": record.rto_minutes,
            "last_met": record.last_met,
            "recorded_at": self.clock.now().to_rfc3339(),
        });
        let bytes = serde_json::to_vec_pretty(&body).map_err(serialization_error)?;
        let path = self.root.join("rpo-rto.json");
        let tmp = self.root.join("rpo-rto.json.tmp");
        if tmp.exists() {
            let _ = fs::remove_file(&tmp);
        }
        write_new_file(&tmp, &bytes)?;
        fs::rename(&tmp, &path).map_err(|error| {
            SemanticError::Io(format!(
                "failed to atomically rewrite rpo-rto.json at {}: {error}",
                path.display()
            ))
        })?;
        Ok(record)
    }

    /// Reads back the most recent `record_rpo_rto` write. Returns `Ok(None)`
    /// if `<root>/rpo-rto.json` does not exist (a fresh store has no recorded
    /// objectives yet). A present-but-unparseable file is `Err` rather than
    /// `None` — the operator wrote SOMETHING, and silently treating it as
    /// "no objectives" would hide corruption from the dashboard.
    pub fn read_rpo_rto(&self) -> Result<Option<crate::recovery::RpoRto>> {
        let path = self.root.join("rpo-rto.json");
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = fs::read(&path).map_err(io_error)?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(serialization_error)?;
        let rpo_minutes = value
            .get("rpo_minutes")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                SemanticError::Serialization("rpo-rto.json missing rpo_minutes".to_owned())
            })? as u32;
        let rto_minutes = value
            .get("rto_minutes")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                SemanticError::Serialization("rpo-rto.json missing rto_minutes".to_owned())
            })? as u32;
        let last_met = value
            .get("last_met")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| {
                SemanticError::Serialization("rpo-rto.json missing last_met".to_owned())
            })?;
        Ok(Some(crate::recovery::RpoRto {
            rpo_minutes,
            rto_minutes,
            last_met,
        }))
    }

    #[cfg(feature = "semantic-test-failpoints")]
    pub fn forge_context_for_test(&self, client_id: Uuid) -> TrustedContext {
        TrustedContext {
            store_uuid: self.marker.store_uuid,
            owner_id: self.marker.owner_id,
            actor_id: self.marker.actor_id,
            client_id,
        }
    }

    pub fn capture(
        &self,
        context: &TrustedContext,
        command: CaptureCommand,
    ) -> Result<MutationOutcome> {
        validate_capture_limits(&command, self.max_object_bytes)?;
        let request_hash = request_hash("capture", &command)?;
        let media_type = command.media_type.clone();
        let bytes = command.bytes.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            None,
            move |_, _identity| {
                Ok(MutationMaterial {
                    event_type: "source_captured",
                    object_bytes: bytes.clone(),
                    media_type: media_type.clone(),
                    generated: GeneratedIds {
                        source_id: Some(Uuid::now_v7()),
                        rendition_id: Some(Uuid::now_v7()),
                        evidence_id: Some(Uuid::now_v7()),
                        ..GeneratedIds::default()
                    },
                })
            },
        )
    }

    /// Read back the plaintext bytes of a previously captured source
    /// (Task D3). This is the read half of [`Self::capture`] — an
    /// extraction worker quarantines a source, then reads it back here to
    /// build a provider request (the bytes are DATA to the AI provider,
    /// never instructions — see [`crate::extraction::ExtractionPolicy`]).
    /// Scoped to `context`'s client, matching the existing per-client
    /// `operation_id` isolation every other mutation resolves under (a
    /// worker cannot read another client's capture by guessing its
    /// operation_id).
    pub fn read_capture(&self, context: &TrustedContext, operation_id: &str) -> Result<Vec<u8>> {
        validate_context(&self.marker, context)?;
        let connection = open_connection(&self.root)?;
        let bytes: Option<Vec<u8>> = connection
            .query_row(
                "SELECT outcome FROM operations WHERE owner_id=?1 AND client_id=?2 AND operation_id=?3",
                params![
                    context.owner_id.to_string(),
                    context.client_id.to_string(),
                    operation_id
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error)?
            .flatten();
        let bytes =
            bytes.ok_or_else(|| SemanticError::MissingDependency(operation_id.to_owned()))?;
        let outcome: MutationOutcome =
            serde_json::from_slice(&bytes).map_err(serialization_error)?;
        decrypt_object(&connection, &self.root, &outcome.event.payload.object_id)
    }

    pub fn propose(
        &self,
        context: &TrustedContext,
        command: ProposeCommand,
    ) -> Result<MutationOutcome> {
        validate_claim_draft(&command.draft)?;
        let request_hash = request_hash("propose", &command)?;
        let capture_operation = command.capture_operation_id.clone();
        let draft = command.draft.clone();
        let root = self.root.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            None,
            move |transaction, _identity| {
                let captured = stored_outcome(transaction, context, &capture_operation)?;
                let proposal_id = Uuid::now_v7();
                let source_object_id = captured.event.payload.object_id;
                let source_bytes = decrypt_object(transaction, &root, &source_object_id)?;
                if source_bytes.is_empty() {
                    return Err(SemanticError::InvalidClaim(
                        "captured rendition is empty, so no evidence span can exist".to_owned(),
                    ));
                }
                if std::str::from_utf8(&source_bytes).is_err() {
                    return Err(SemanticError::InvalidClaim(
                        "captured rendition is not valid UTF-8".to_owned(),
                    ));
                }
                let quote_hash = source_object_id
                    .strip_prefix("sha256:")
                    .ok_or_else(|| {
                        SemanticError::CorruptLedger("capture object has invalid ID".to_owned())
                    })?
                    .to_owned();
                let provenance = Provenance::Evidence {
                    source_id: captured.generated.source_id.ok_or_else(|| {
                        SemanticError::MissingDependency(capture_operation.clone())
                    })?,
                    rendition_id: captured.generated.rendition_id.ok_or_else(|| {
                        SemanticError::MissingDependency(capture_operation.clone())
                    })?,
                    evidence_id: captured.generated.evidence_id.ok_or_else(|| {
                        SemanticError::MissingDependency(capture_operation.clone())
                    })?,
                    object_id: source_object_id.clone(),
                    byte_start: 0,
                    byte_end: u64::try_from(source_bytes.len()).map_err(|_| {
                        SemanticError::CorruptLedger("capture object is too large".to_owned())
                    })?,
                    quote_hash,
                };
                let proposal = ProposalObject {
                    kind: "claim_proposal".to_owned(),
                    proposal_id,
                    source_object_id,
                    provenance,
                    draft: draft.clone(),
                };
                insert_proposal_status(transaction, proposal_id)?;
                Ok(MutationMaterial {
                    event_type: "claim_proposed",
                    object_bytes: canonical_bytes(&proposal)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds {
                        proposal_id: Some(proposal_id),
                        ..GeneratedIds::default()
                    },
                })
            },
        )
    }

    /// Proposes a claim derived by inference rather than direct evidence. An
    /// inference with no evidence captures is stored as `unsupported=true`
    /// and can be proposed/rejected but never confirmed (ADR Decision 6).
    pub fn propose_inference(
        &self,
        context: &TrustedContext,
        command: ProposeInferenceCommand,
    ) -> Result<MutationOutcome> {
        validate_claim_draft(&command.draft)?;
        let request_hash = request_hash("propose_inference", &command)?;
        let evidence_captures = command.evidence_capture_operation_ids.clone();
        let method = command.method.clone();
        let model = command.model.clone();
        let prompt_version = command.prompt_version.clone();
        let draft = command.draft.clone();
        let root = self.root.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("propose"),
            move |transaction, _identity| {
                let proposal_id = Uuid::now_v7();
                let mut evidence = Vec::with_capacity(evidence_captures.len());
                for capture_operation in &evidence_captures {
                    let captured = stored_outcome(transaction, context, capture_operation)?;
                    let source_object_id = captured.event.payload.object_id;
                    let source_bytes = decrypt_object(transaction, &root, &source_object_id)?;
                    let quote_hash = source_object_id
                        .strip_prefix("sha256:")
                        .ok_or_else(|| {
                            SemanticError::CorruptLedger("capture object has invalid ID".to_owned())
                        })?
                        .to_owned();
                    evidence.push(InferenceEvidenceSpan {
                        source_id: captured.generated.source_id.ok_or_else(|| {
                            SemanticError::MissingDependency(capture_operation.clone())
                        })?,
                        rendition_id: captured.generated.rendition_id.ok_or_else(|| {
                            SemanticError::MissingDependency(capture_operation.clone())
                        })?,
                        evidence_id: captured.generated.evidence_id.ok_or_else(|| {
                            SemanticError::MissingDependency(capture_operation.clone())
                        })?,
                        object_id: source_object_id,
                        byte_start: 0,
                        byte_end: u64::try_from(source_bytes.len()).map_err(|_| {
                            SemanticError::CorruptLedger("capture object is too large".to_owned())
                        })?,
                        quote_hash,
                    });
                }
                let unsupported = evidence.is_empty();
                let proposal = ProposalObject {
                    kind: "claim_proposal".to_owned(),
                    proposal_id,
                    source_object_id: String::new(),
                    provenance: Provenance::Inference {
                        method: method.clone(),
                        model: model.clone(),
                        prompt_version: prompt_version.clone(),
                        evidence,
                        unsupported,
                    },
                    draft: draft.clone(),
                };
                insert_proposal_status(transaction, proposal_id)?;
                Ok(MutationMaterial {
                    event_type: "claim_proposed",
                    object_bytes: canonical_bytes(&proposal)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds {
                        proposal_id: Some(proposal_id),
                        ..GeneratedIds::default()
                    },
                })
            },
        )
    }

    pub fn confirm(
        &self,
        context: &TrustedContext,
        command: ConfirmCommand,
    ) -> Result<MutationOutcome> {
        let request_hash = request_hash("confirm", &command)?;
        let proposal_operation = command.proposal_operation_id.clone();
        let root = self.root.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, identity| {
                finish_confirmation(
                    transaction,
                    context,
                    &root,
                    &proposal_operation,
                    identity,
                    &[],
                )
            },
        )
    }

    /// Confirms a proposal as a claim that supersedes one or more
    /// previously confirmed claims in the same `(domain, subject, predicate)`
    /// scope. The superseded claims' history is kept; they simply stop being
    /// current as of this event (ADR Decision 6, "no silent overwrite").
    pub fn supersede(
        &self,
        context: &TrustedContext,
        command: SupersedeCommand,
    ) -> Result<MutationOutcome> {
        if command.superseded_claim_operation_ids.is_empty() {
            return Err(SemanticError::InvalidClaim(
                "supersede requires at least one prior claim operation_id".to_owned(),
            ));
        }
        let request_hash = request_hash("supersede", &command)?;
        let proposal_operation = command.proposal_operation_id.clone();
        let superseded_operations = command.superseded_claim_operation_ids.clone();
        let root = self.root.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, identity| {
                finish_confirmation(
                    transaction,
                    context,
                    &root,
                    &proposal_operation,
                    identity,
                    &superseded_operations,
                )
            },
        )
    }

    /// Rejects a proposal without ever creating a claim from it.
    pub fn reject(
        &self,
        context: &TrustedContext,
        command: RejectCommand,
    ) -> Result<MutationOutcome> {
        let request_hash = request_hash("reject", &command)?;
        let proposal_operation = command.proposal_operation_id.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, _identity| {
                let proposed = stored_outcome(transaction, context, &proposal_operation)?;
                let proposal_id = proposed.generated.proposal_id.ok_or_else(|| {
                    SemanticError::InvalidTransition(format!(
                        "operation {proposal_operation} did not propose a claim"
                    ))
                })?;
                let status: String = transaction
                    .query_row(
                        "SELECT status FROM proposal_status WHERE proposal_id=?1",
                        [proposal_id.to_string()],
                        |row| row.get(0),
                    )
                    .map_err(database_error)?;
                if status != "proposed" {
                    return Err(SemanticError::InvalidTransition(format!(
                        "proposal {proposal_id} is already {status}, cannot reject"
                    )));
                }
                transaction
                    .execute(
                        "UPDATE proposal_status SET status='rejected' WHERE proposal_id=?1",
                        [proposal_id.to_string()],
                    )
                    .map_err(database_error)?;
                let rejection = RejectionObject {
                    kind: "claim_rejection".to_owned(),
                    proposal_id,
                };
                Ok(MutationMaterial {
                    event_type: "claim_rejected",
                    object_bytes: canonical_bytes(&rejection)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds::default(),
                })
            },
        )
    }

    /// Confirms a proposal identified by `proposal_id` (Task 5.1 Inbox). See
    /// [`ConfirmByProposalIdCommand`] — the caller need not be the original
    /// proposer.
    pub fn confirm_by_proposal_id(
        &self,
        context: &TrustedContext,
        command: ConfirmByProposalIdCommand,
    ) -> Result<MutationOutcome> {
        let request_hash = request_hash("confirm_by_proposal_id", &command)?;
        let proposal_id = command.proposal_id;
        let owner_id = context.owner_id;
        let root = self.root.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, identity| {
                finish_confirmation_by_id(transaction, &root, owner_id, proposal_id, identity, &[])
            },
        )
    }

    /// Confirms a proposal identified by `proposal_id`, superseding prior
    /// claims identified directly by `claim_id` (Task 5.1 Inbox). See
    /// [`SupersedeByProposalIdCommand`].
    pub fn supersede_by_proposal_id(
        &self,
        context: &TrustedContext,
        command: SupersedeByProposalIdCommand,
    ) -> Result<MutationOutcome> {
        if command.superseded_claim_ids.is_empty() {
            return Err(SemanticError::InvalidClaim(
                "supersede requires at least one prior claim id".to_owned(),
            ));
        }
        let request_hash = request_hash("supersede_by_proposal_id", &command)?;
        let proposal_id = command.proposal_id;
        let superseded_claim_ids = command.superseded_claim_ids.clone();
        let owner_id = context.owner_id;
        let root = self.root.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, identity| {
                finish_confirmation_by_id(
                    transaction,
                    &root,
                    owner_id,
                    proposal_id,
                    identity,
                    &superseded_claim_ids,
                )
            },
        )
    }

    /// Rejects a proposal identified by `proposal_id` (Task 5.1 Inbox). See
    /// [`RejectByProposalIdCommand`] — the caller need not be the original
    /// proposer.
    pub fn reject_by_proposal_id(
        &self,
        context: &TrustedContext,
        command: RejectByProposalIdCommand,
    ) -> Result<MutationOutcome> {
        let request_hash = request_hash("reject_by_proposal_id", &command)?;
        let proposal_id = command.proposal_id;
        let owner_id = context.owner_id;
        let root = self.root.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, _identity| {
                let _ = resolve_proposal_by_id(transaction, &root, owner_id, proposal_id)?;
                let status: String = transaction
                    .query_row(
                        "SELECT status FROM proposal_status WHERE proposal_id=?1",
                        [proposal_id.to_string()],
                        |row| row.get(0),
                    )
                    .map_err(database_error)?;
                if status != "proposed" {
                    return Err(SemanticError::InvalidTransition(format!(
                        "proposal {proposal_id} is already {status}, cannot reject"
                    )));
                }
                transaction
                    .execute(
                        "UPDATE proposal_status SET status='rejected' WHERE proposal_id=?1",
                        [proposal_id.to_string()],
                    )
                    .map_err(database_error)?;
                let rejection = RejectionObject {
                    kind: "claim_rejection".to_owned(),
                    proposal_id,
                };
                Ok(MutationMaterial {
                    event_type: "claim_rejected",
                    object_bytes: canonical_bytes(&rejection)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds::default(),
                })
            },
        )
    }

    /// Retracts a previously confirmed claim. History and evidence are kept;
    /// the claim simply stops being current as of this event.
    pub fn retract(
        &self,
        context: &TrustedContext,
        command: RetractCommand,
    ) -> Result<MutationOutcome> {
        let request_hash = request_hash("retract", &command)?;
        let claim_operation = command.claim_operation_id.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, identity| {
                let confirmed = stored_outcome(transaction, context, &claim_operation)?;
                let claim_id = confirmed.generated.claim_id.ok_or_else(|| {
                    SemanticError::InvalidTransition(format!(
                        "operation {claim_operation} did not confirm a claim"
                    ))
                })?;
                let retracted_at: Option<Option<i64>> = transaction
                    .query_row(
                        "SELECT retracted_at_event_seq FROM claim_status WHERE claim_id=?1",
                        [claim_id.to_string()],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(database_error)?;
                let Some(retracted_at) = retracted_at else {
                    return Err(SemanticError::InvalidTransition(format!(
                        "claim {claim_id} is not a known confirmed claim"
                    )));
                };
                if retracted_at.is_some() {
                    return Err(SemanticError::InvalidTransition(format!(
                        "claim {claim_id} is already retracted"
                    )));
                }
                transaction
                    .execute(
                        "UPDATE claim_status SET retracted_at_event_seq=?2 WHERE claim_id=?1",
                        params![claim_id.to_string(), identity.event_seq as i64],
                    )
                    .map_err(database_error)?;
                let retraction = RetractionObject {
                    kind: "claim_retraction".to_owned(),
                    claim_id,
                };
                Ok(MutationMaterial {
                    event_type: "claim_retracted",
                    object_bytes: canonical_bytes(&retraction)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds::default(),
                })
            },
        )
    }

    // ── Task 2.2: human-authored + mechanical proposal paths ────────────────

    /// Propose a claim the owner asserted directly (Task 2.2 — human edit
    /// becomes an authored event). The utterance bytes ARE the evidence, so
    /// this path mints a `Provenance::UserAssertion` instead of the external
    /// `Provenance::Evidence` that `propose` produces. ADR Decision 6.
    pub fn propose_user_assertion(
        &self,
        context: &TrustedContext,
        command: ProposeUserAssertionCommand,
    ) -> Result<MutationOutcome> {
        validate_claim_draft(&command.draft)?;
        if command.utterance.is_empty() {
            return Err(SemanticError::InvalidClaim(
                "user assertion utterance is empty".to_owned(),
            ));
        }
        let request_hash = request_hash("propose_user_assertion", &command)?;
        let utterance = command.utterance.clone();
        let draft = command.draft.clone();
        let actor_id = context.actor_id;
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            None,
            move |transaction, _identity| {
                let proposal_id = Uuid::now_v7();
                // Capture the utterance as a content-addressed object inside
                // the same transaction, exactly like `capture` does, so the
                // user_assertion spans point at real bytes.
                let utterance_object_id = publish_object(&self.root, transaction, &utterance)?;
                let utterance_byte_end = u64::try_from(utterance.len()).map_err(|_| {
                    SemanticError::CorruptLedger("utterance is too large".to_owned())
                })?;
                let proposal = ProposalObject {
                    kind: "claim_proposal".to_owned(),
                    proposal_id,
                    source_object_id: utterance_object_id.clone(),
                    provenance: Provenance::UserAssertion {
                        actor_id,
                        utterance_object_id,
                        utterance_byte_start: 0,
                        utterance_byte_end,
                    },
                    draft: draft.clone(),
                };
                insert_proposal_status(transaction, proposal_id)?;
                Ok(MutationMaterial {
                    event_type: "claim_proposed",
                    object_bytes: canonical_bytes(&proposal)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds {
                        proposal_id: Some(proposal_id),
                        ..GeneratedIds::default()
                    },
                })
            },
        )
    }

    /// Propose a mechanically-derived claim (Task 2.2). The derivation is
    /// reproducible from `input_hashes` + `method`/`method_version`, pinned by
    /// `output_hash`. ADR Decision 6's `mechanical` variant.
    pub fn propose_mechanical(
        &self,
        context: &TrustedContext,
        command: ProposeMechanicalCommand,
    ) -> Result<MutationOutcome> {
        validate_claim_draft(&command.draft)?;
        if command.method.is_empty() || command.method_version.is_empty() {
            return Err(SemanticError::InvalidClaim(
                "mechanical provenance requires non-empty method and method_version".to_owned(),
            ));
        }
        let request_hash = request_hash("propose_mechanical", &command)?;
        let method = command.method.clone();
        let method_version = command.method_version.clone();
        let input_hashes = command.input_hashes.clone();
        let output_hash = command.output_hash.clone();
        let draft = command.draft.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            None,
            move |transaction, _identity| {
                let proposal_id = Uuid::now_v7();
                let proposal = ProposalObject {
                    kind: "claim_proposal".to_owned(),
                    proposal_id,
                    source_object_id: String::new(),
                    provenance: Provenance::Mechanical {
                        method: method.clone(),
                        method_version: method_version.clone(),
                        input_hashes: input_hashes.clone(),
                        output_hash: output_hash.clone(),
                    },
                    draft: draft.clone(),
                };
                insert_proposal_status(transaction, proposal_id)?;
                Ok(MutationMaterial {
                    event_type: "claim_proposed",
                    object_bytes: canonical_bytes(&proposal)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds {
                        proposal_id: Some(proposal_id),
                        ..GeneratedIds::default()
                    },
                })
            },
        )
    }

    // ── Task 2.2: entity model public read/resolve API ──────────────────────

    /// Resolve `(domain, subject)` to a stable entity_id, minting a new
    /// UUIDv7 entity if none exists yet. Idempotent: the same tuple always
    /// resolves to the same id. Resolution checks aliases first, so former
    /// subjects (post-rename/merge) keep resolving. ADR Decision 3.
    pub fn resolve_or_create_entity(
        &self,
        context: &TrustedContext,
        domain: &str,
        subject: &str,
    ) -> Result<Uuid> {
        validate_context(&self.marker, context)?;
        // Take the in-process writer lock BEFORE opening the Immediate
        // transaction, matching `mutate_once`'s ordering: without it, two
        // threads could both BEGIN IMMEDIATE on separate connections and the
        // loser would surface a DatabaseContention error with no retry loop.
        let _writer = self.coordinator.writer.lock();
        let mut connection = open_connection(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let entity_id = resolve_or_create_entity_in_tx(
            &transaction,
            domain,
            subject,
            next_event_seq(&transaction, context.owner_id)?,
        )?;
        transaction.commit().map_err(database_error)?;
        Ok(entity_id)
    }

    /// Resolve `(domain, alias)` to an entity_id without creating one. Returns
    /// `Err(MissingDependency)` if no entity has ever held this alias — callers
    /// that need create-on-demand semantics use [`Self::resolve_or_create_entity`].
    pub fn resolve_entity(
        &self,
        context: &TrustedContext,
        domain: &str,
        alias: &str,
    ) -> Result<Uuid> {
        validate_context(&self.marker, context)?;
        let _maintenance = self.coordinator.maintenance.read();
        let connection = open_connection(&self.root)?;
        resolve_entity_in_tx(&connection, domain, alias)?
            .ok_or_else(|| SemanticError::MissingDependency(format!("entity {domain}/{alias}")))
    }

    /// Read the canonical subject + identity of an entity by its stable id.
    pub fn entity_by_id(&self, context: &TrustedContext, entity_id: Uuid) -> Result<EntityRecord> {
        validate_context(&self.marker, context)?;
        let _maintenance = self.coordinator.maintenance.read();
        let connection = open_connection(&self.root)?;
        let row: Option<(String, String, String)> = connection
            .query_row(
                "SELECT entity_id, canonical_subject, created_at FROM entities WHERE entity_id=?1",
                [entity_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(database_error)?;
        let (_, canonical_subject, created_at) =
            row.ok_or_else(|| SemanticError::MissingDependency(format!("entity {entity_id}")))?;
        let parsed_created = created_at.parse().map_err(|_| {
            SemanticError::CorruptLedger("entity created_at is not RFC 3339".to_owned())
        })?;
        // Recover the domain via the canonical alias row.
        let domain: String = connection
            .query_row(
                "SELECT domain FROM entity_aliases WHERE entity_id=?1 AND kind='canonical' ORDER BY aliased_at_event_seq DESC LIMIT 1",
                [entity_id.to_string()],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        Ok(EntityRecord {
            entity_id,
            domain,
            canonical_subject,
            created_at: parsed_created,
        })
    }

    /// Rename an entity's canonical subject (Task 2.2 DoD bullet 3). Emits an
    /// `entity_renamed` event, updates `entities.canonical_subject`, and
    /// records the OLD subject as a `former_subject` alias so existing
    /// references keep resolving (backlink preservation). The new subject must
    /// not collide with another entity in the same domain — that is a merge.
    pub fn rename_entity(
        &self,
        context: &TrustedContext,
        command: RenameEntityCommand,
    ) -> Result<MutationOutcome> {
        let request_hash = request_hash("rename_entity", &command)?;
        let entity_id = command.entity_id;
        let new_subject = command.new_subject.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, identity| {
                // Load current canonical row.
                let row: Option<(String, String)> = transaction
                    .query_row(
                        "SELECT domain, canonical_subject FROM entities WHERE entity_id=?1",
                        [entity_id.to_string()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(database_error)?;
                let (domain, old_subject) = row.ok_or_else(|| {
                    SemanticError::MissingDependency(format!("entity {entity_id}"))
                })?;
                if old_subject == new_subject {
                    return Err(SemanticError::InvalidTransition(format!(
                        "entity {entity_id} canonical subject is already {new_subject}"
                    )));
                }
                // Collision check: a *different* entity already owns this subject.
                let collision: Option<String> = transaction
                    .query_row(
                        "SELECT entity_id FROM entities WHERE domain=?1 AND canonical_subject=?2 AND entity_id<>?3",
                        params![domain, new_subject, entity_id.to_string()],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(database_error)?;
                if collision.is_some() {
                    return Err(SemanticError::InvalidTransition(format!(
                        "subject {new_subject} in domain {domain} is already canonical for a different entity; use merge instead"
                    )));
                }
                transaction
                    .execute(
                        "UPDATE entities SET canonical_subject=?2 WHERE entity_id=?1",
                        params![entity_id.to_string(), new_subject.clone()],
                    )
                    .map_err(database_error)?;
                // Record the old subject as a former_subject alias (backlink)
                // and the new subject as canonical.
                insert_alias(transaction, &domain, &old_subject, entity_id, "former_subject", identity.event_seq)?;
                insert_alias(transaction, &domain, &new_subject, entity_id, "canonical", identity.event_seq)?;
                let payload = serde_json::json!({
                    "kind": "entity_renamed",
                    "entity_id": entity_id,
                    "domain": domain,
                    "old_subject": old_subject,
                    "new_subject": new_subject,
                });
                Ok(MutationMaterial {
                    event_type: "entity_renamed",
                    object_bytes: canonical_bytes(&payload)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds::default(),
                })
            },
        )
    }

    /// Merge the source entity into the target (Task 2.2 DoD bullet 3). Emits
    /// an `entity_merged` event, rewrites every `claim_status.entity_id` from
    /// source onto target, and turns the source's canonical subject (plus its
    /// prior aliases) into backlinks pointing at the target. No claim loses
    /// its entity reference; no alias is deleted.
    pub fn merge_entities(
        &self,
        context: &TrustedContext,
        command: MergeEntitiesCommand,
    ) -> Result<MutationOutcome> {
        if command.source_entity_id == command.target_entity_id {
            return Err(SemanticError::InvalidTransition(format!(
                "cannot merge entity {} into itself",
                command.source_entity_id
            )));
        }
        let request_hash = request_hash("merge_entities", &command)?;
        let source = command.source_entity_id;
        let target = command.target_entity_id;
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, identity| {
                let source_row: Option<(String, String)> = transaction
                    .query_row(
                        "SELECT domain, canonical_subject FROM entities WHERE entity_id=?1",
                        [source.to_string()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(database_error)?;
                let (source_domain, source_subject) = source_row.ok_or_else(|| {
                    SemanticError::MissingDependency(format!("entity {source}"))
                })?;
                let target_row: Option<(String, String)> = transaction
                    .query_row(
                        "SELECT domain, canonical_subject FROM entities WHERE entity_id=?1",
                        [target.to_string()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(database_error)?;
                let (target_domain, _target_subject) = target_row.ok_or_else(|| {
                    SemanticError::MissingDependency(format!("entity {target}"))
                })?;
                if source_domain != target_domain {
                    return Err(SemanticError::InvalidTransition(format!(
                        "cannot merge across domains: source={source_domain}, target={target_domain}"
                    )));
                }
                // Rewrite every claim attached to the source onto the target.
                let moved = transaction
                    .execute(
                        "UPDATE claim_status SET entity_id=?2 WHERE entity_id=?1",
                        params![source.to_string(), target.to_string()],
                    )
                    .map_err(database_error)?;
                // Fold the source's canonical subject + all its aliases onto the
                // target so every historical reference keeps resolving.
                let mut alias_statement = transaction
                    .prepare("SELECT alias FROM entity_aliases WHERE entity_id=?1")
                    .map_err(database_error)?;
                let alias_rows = alias_statement
                    .query_map([source.to_string()], |row| row.get::<_, String>(0))
                    .map_err(database_error)?;
                let mut aliases = Vec::new();
                for alias_row in alias_rows {
                    aliases.push(alias_row.map_err(database_error)?);
                }
                drop(alias_statement);
                aliases.push(source_subject.clone());
                for alias in &aliases {
                    insert_alias(transaction, &source_domain, alias, target, "former_subject", identity.event_seq)?;
                }
                // Remove the source's canonical row: its claims were already
                // rewritten onto the target, and every alias (including its
                // former canonical subject) now points at the target with a
                // higher `aliased_at_event_seq`, so `resolve_entity_in_tx`'s
                // `ORDER BY ... DESC LIMIT 1` resolves the target. The merge
                // event itself is the audit record — the source entity_id is
                // captured in the event payload below.
                transaction
                    .execute(
                        "DELETE FROM entities WHERE entity_id=?1",
                        [source.to_string()],
                    )
                    .map_err(database_error)?;
                let payload = serde_json::json!({
                    "kind": "entity_merged",
                    "domain": source_domain,
                    "source_entity_id": source,
                    "target_entity_id": target,
                    "claims_moved": moved,
                });
                Ok(MutationMaterial {
                    event_type: "entity_merged",
                    object_bytes: canonical_bytes(&payload)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds::default(),
                })
            },
        )
    }

    /// Enumerate every claim attached to an entity (Task 2.2). Used by the
    /// merge test to prove no claim is orphaned, and by future Console views.
    pub fn claims_for_entity(
        &self,
        context: &TrustedContext,
        entity_id: Uuid,
    ) -> Result<Vec<ClaimView>> {
        validate_context(&self.marker, context)?;
        let _maintenance = self.coordinator.maintenance.read();
        let connection = open_connection(&self.root)?;
        let mut statement = connection
            .prepare("SELECT claim_id, confirmed_event_seq FROM claim_status WHERE entity_id=?1")
            .map_err(database_error)?;
        let rows = statement
            .query_map([entity_id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(database_error)?;
        let mut out = Vec::new();
        for row in rows {
            let (claim_id_text, confirmed_event_seq) = row.map_err(database_error)?;
            let object_id: String = connection
                .query_row(
                    "SELECT object_id FROM events WHERE owner_id=?1 AND event_seq=?2",
                    params![self.marker.owner_id.to_string(), confirmed_event_seq],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            let object: ConfirmationObject =
                serde_json::from_slice(&decrypt_object(&connection, &self.root, &object_id)?)
                    .map_err(serialization_error)?;
            let seq = u64::try_from(confirmed_event_seq)
                .map_err(|_| SemanticError::CorruptLedger("negative event sequence".to_owned()))?;
            out.push(build_claim_view(object.claim, seq, Some(&claim_id_text)));
        }
        Ok(out)
    }

    /// Snapshot of every entity's current canonical subject, keyed by
    /// `entity_id` text. Used by the projection layer so a renamed entity's
    /// generated page reflects the *current* canonical subject rather than the
    /// subject string captured at confirm time — the claim payload is an
    /// immutable historical record, but the generated wiki is a live view
    /// (ADR Decision 1: generated wiki is a replaceable materialized view).
    pub fn entity_canonical_subjects(
        &self,
        context: &TrustedContext,
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        validate_context(&self.marker, context)?;
        let _maintenance = self.coordinator.maintenance.read();
        let connection = open_connection(&self.root)?;
        let mut statement = connection
            .prepare("SELECT entity_id, canonical_subject FROM entities")
            .map_err(database_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database_error)?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (entity_id_text, canonical_subject) = row.map_err(database_error)?;
            if let Ok(entity_id) = Uuid::parse_str(&entity_id_text) {
                out.insert(entity_id, canonical_subject);
            }
        }
        Ok(out)
    }

    /// Owner-scoped variant of [`Self::entity_canonical_subjects`] for callers
    /// (like the projection adapter) that only hold a `&SemanticStore` and use
    /// the bootstrap trusted context. Identical read path.
    pub fn entity_canonical_subjects_owned(&self) -> std::collections::HashMap<Uuid, String> {
        self.entity_canonical_subjects(&self.trusted_context())
            .unwrap_or_default()
    }

    /// Backfill `claim_status.entity_id` for every confirmed row that is
    /// currently NULL (Task 2.3). Each row is classified:
    ///
    /// - `migrated` — row had NULL entity_id and `(domain, subject)` resolved
    ///   to an existing entity (via `entity_aliases`); the binding is written.
    /// - `skipped` — row already had a non-NULL entity_id (no work).
    /// - `ambiguous` — row had NULL entity_id and `(domain, subject)` did not
    ///   resolve to any existing entity. Backfill is **resolve-only**: it does
    ///   not lazily create entities, so an orphan row (e.g. from a partial
    ///   import with no matching entity) stays unbound rather than being
    ///   silently bound to a freshly-minted entity.
    /// - `error` — the row's `claim_id` failed to parse as a UUID (corrupt row).
    ///
    /// When `dry_run` is true, no writes occur — every `migrated` candidate
    /// is reported as if it would be migrated, but the binding is not
    /// persisted. Reruns are idempotent: rows migrated on a previous run are
    /// `skipped` on the next.
    ///
    /// This path only populates a metadata column on already-confirmed
    /// claims; it never creates new claims, never promotes a proposal, and
    /// never touches claim payloads (subject/predicate/value/provenance). An
    /// LLM-derived claim must still enter the store via `propose_inference`,
    /// which emits `claim_proposed` (status `proposed`) — §5 Memory Policy.
    pub fn backfill_entity_ids(
        &self,
        context: &TrustedContext,
        dry_run: bool,
    ) -> Result<MigrationReport> {
        validate_context(&self.marker, context)?;
        let _maintenance = self.coordinator.maintenance.read();
        let _writer = self.coordinator.writer.lock();
        let mut connection = open_connection(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;

        let mut statement = transaction
            .prepare(
                "SELECT claim_id, domain, subject, predicate, entity_id
                 FROM claim_status",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(database_error)?;
        // Collect first so we can drop the statement borrow before writing.
        let mut examined: Vec<(String, String, String, String, Option<String>)> = Vec::new();
        for row in rows {
            examined.push(row.map_err(database_error)?);
        }
        drop(statement);

        let mut report = MigrationReport {
            migrated: 0,
            skipped: 0,
            ambiguous: 0,
            error: 0,
            records: Vec::with_capacity(examined.len()),
        };
        for (claim_id_text, domain, subject, predicate, existing) in examined {
            let claim_id = match Uuid::parse_str(&claim_id_text) {
                Ok(id) => id,
                Err(_) => {
                    report.error += 1;
                    report.records.push(MigrationRecord {
                        claim_id: Uuid::nil(),
                        domain,
                        subject,
                        predicate,
                        outcome: "error".to_owned(),
                        entity_id: None,
                        detail: Some(format!("claim_id {claim_id_text} is not a UUID")),
                    });
                    continue;
                }
            };
            if existing.is_some() {
                report.skipped += 1;
                report.records.push(MigrationRecord {
                    claim_id,
                    domain,
                    subject,
                    predicate,
                    outcome: "skipped".to_owned(),
                    entity_id: existing.and_then(|t| Uuid::parse_str(&t).ok()),
                    detail: None,
                });
                continue;
            }
            // NULL entity_id — resolve against existing entities/aliases only.
            // Backfill does NOT lazily create entities: a row that has no
            // matching entity is `ambiguous` (the migration does not guess
            // which entity an orphan row should bind to). Real confirmed
            // claims always have a matching entity because Task 2.2's
            // confirm path created one; only corrupt/orphan rows fail here.
            match resolve_entity_in_tx(&transaction, &domain, &subject)? {
                Some(entity_id) => {
                    if !dry_run {
                        transaction
                            .execute(
                                "UPDATE claim_status SET entity_id=?2 WHERE claim_id=?1",
                                params![claim_id_text, entity_id.to_string()],
                            )
                            .map_err(database_error)?;
                    }
                    report.migrated += 1;
                    report.records.push(MigrationRecord {
                        claim_id,
                        domain,
                        subject,
                        predicate,
                        outcome: "migrated".to_owned(),
                        entity_id: Some(entity_id),
                        detail: None,
                    });
                }
                None => {
                    report.ambiguous += 1;
                    let detail = format!("no entity matches {domain}/{subject}; will not guess");
                    report.records.push(MigrationRecord {
                        claim_id,
                        domain,
                        subject,
                        predicate,
                        outcome: "ambiguous".to_owned(),
                        entity_id: None,
                        detail: Some(detail),
                    });
                }
            }
        }
        if !dry_run {
            transaction.commit().map_err(database_error)?;
        }
        Ok(report)
    }

    // ── Task 2.3 test helpers ──────────────────────────────────────────────
    // These exist behind no feature gate because the backfill tests need to
    // simulate legacy/corrupt state that cannot be produced through the public
    // API. They are plain methods (not #[cfg(test)]) so integration tests in
    // tests/ can call them; they are documented as test-only and would be
    // removed or feature-gated before any production release.

    /// Test helper: set `claim_status.entity_id = NULL` for the row matching
    /// `(domain, subject, predicate)`, simulating a legacy row from before
    /// Task 2.2's confirm-time binding.
    pub fn null_entity_id_for_test(&self, domain: &str, subject: &str, predicate: &str) {
        let _maintenance = self.coordinator.maintenance.read();
        let _writer = self.coordinator.writer.lock();
        let connection = open_connection(&self.root).expect("open");
        connection
            .execute(
                "UPDATE claim_status SET entity_id=NULL WHERE domain=?1 AND subject=?2 AND predicate=?3",
                params![domain, subject, predicate],
            )
            .expect("null entity_id");
    }

    /// Test helper: null every `claim_status.entity_id`, simulating a full
    /// pre-Task-2.2 store.
    pub fn null_all_entity_ids_for_test(&self) {
        let _maintenance = self.coordinator.maintenance.read();
        let _writer = self.coordinator.writer.lock();
        let connection = open_connection(&self.root).expect("open");
        connection
            .execute("UPDATE claim_status SET entity_id=NULL", [])
            .expect("null all entity_ids");
    }

    /// Test helper: insert a bare `claim_status` row with no matching claim
    /// payload and no entity, simulating an orphan from a partial import. The
    /// `confirmed_event_seq` is set to 0 so it does not collide with real
    /// events.
    pub fn insert_orphan_claim_status_for_test(
        &self,
        domain: &str,
        subject: &str,
        predicate: &str,
    ) {
        let _maintenance = self.coordinator.maintenance.read();
        let _writer = self.coordinator.writer.lock();
        let connection = open_connection(&self.root).expect("open");
        connection
            .execute(
                "INSERT INTO claim_status(claim_id,domain,subject,predicate,confirmed_event_seq,superseded_by_event_seq,retracted_at_event_seq,entity_id) VALUES (?1,?2,?3,?4,0,NULL,NULL,NULL)",
                params![Uuid::now_v7().to_string(), domain, subject, predicate],
            )
            .expect("insert orphan");
    }

    fn mutate<F>(
        &self,
        context: &TrustedContext,
        operation_id: &str,
        request_hash: &str,
        required_capability: Option<&str>,
        build: F,
    ) -> Result<MutationOutcome>
    where
        F: Fn(&Transaction<'_>, EventIdentity) -> Result<MutationMaterial>,
    {
        validate_context(&self.marker, context)?;
        validate_operation_id(operation_id)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut backoff = Duration::from_millis(5);
        loop {
            match self.mutate_once(
                context,
                operation_id,
                request_hash,
                required_capability,
                &build,
            ) {
                Err(SemanticError::DatabaseContention(_)) if Instant::now() < deadline => {
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_millis(100));
                }
                result => return result,
            }
        }
    }

    fn mutate_once<F>(
        &self,
        context: &TrustedContext,
        operation_id: &str,
        request_hash: &str,
        required_capability: Option<&str>,
        build: &F,
    ) -> Result<MutationOutcome>
    where
        F: Fn(&Transaction<'_>, EventIdentity) -> Result<MutationMaterial>,
    {
        validate_context(&self.marker, context)?;
        validate_operation_id(operation_id)?;
        let _maintenance = self.coordinator.maintenance.read();
        // SQLite WAL permits concurrent readers but still has one writer. Coordinate
        // in-process handles before opening the write transaction so Windows does
        // not exhaust SQLite's SQLITE_PROTOCOL retry budget under a write stampede.
        let _writer = self.coordinator.writer.lock();
        let mut connection = open_connection(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let active = ActiveTransaction::new(&self.coordinator);

        let registered: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM clients WHERE client_id=?1",
                [context.client_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error)?;
        if registered.is_none() {
            return Err(SemanticError::MissingDependency(
                "client is not registered for this store".to_owned(),
            ));
        }

        if let Some(capability) = required_capability {
            let has_capability: Option<i64> = transaction
                .query_row(
                    "SELECT 1 FROM client_capabilities WHERE client_id=?1 AND capability=?2",
                    params![context.client_id.to_string(), capability],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database_error)?;
            if has_capability.is_none() {
                return Err(SemanticError::CapabilityDenied(format!(
                    "client lacks required capability: {capability}"
                )));
            }
        }

        if let Some((stored_hash, outcome)) = transaction
            .query_row(
                "SELECT request_hash, outcome FROM operations WHERE owner_id=?1 AND client_id=?2 AND operation_id=?3",
                params![context.owner_id.to_string(), context.client_id.to_string(), operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
            )
            .optional()
            .map_err(database_error)?
        {
            if stored_hash != request_hash {
                return Err(SemanticError::IdempotencyConflict);
            }
            let bytes = outcome.ok_or_else(|| SemanticError::CorruptLedger("committed operation has no stored outcome".to_owned()))?;
            let replay = serde_json::from_slice(&bytes).map_err(serialization_error)?;
            transaction.commit().map_err(database_error)?;
            drop(active);
            project_and_ack(&self.root, &self.coordinator)?;
            return Ok(replay);
        }

        transaction
            .execute(
                "INSERT INTO operations(owner_id,client_id,operation_id,request_hash,outcome) VALUES (?1,?2,?3,?4,NULL)",
                params![context.owner_id.to_string(), context.client_id.to_string(), operation_id, request_hash],
            )
            .map_err(database_error)?;
        crash_at("after_idempotency_reservation");

        let event_seq = next_event_seq(&transaction, context.owner_id)?;
        let prior_event_hash = prior_event_hash(&transaction, context.owner_id, event_seq)?;
        let identity = EventIdentity {
            event_id: Uuid::now_v7(),
            event_seq,
        };
        let material = build(&transaction, identity)?;
        let object_id = publish_object(&self.root, &transaction, &material.object_bytes)?;
        let mut event = EventEnvelope {
            schema_version: CURRENT_EVENT_SCHEMA_VERSION,
            event_id: identity.event_id,
            owner_id: context.owner_id,
            event_seq,
            event_type: material.event_type.to_owned(),
            recorded_at: self.clock.now(),
            actor_id: context.actor_id,
            client_id: context.client_id,
            operation_id: operation_id.to_owned(),
            request_hash: request_hash.to_owned(),
            payload: ObjectPayload {
                kind: "object_ref".to_owned(),
                object_id: object_id.clone(),
                media_type: Some(material.media_type),
            },
            prior_event_hash,
            event_hash: String::new(),
            purge_epoch: 0,
        };
        event.event_hash = calculate_event_hash(&event)?;
        let event_bytes = canonical_bytes(&event)?;
        transaction
            .execute(
                "INSERT INTO events(owner_id,event_seq,event_id,event_type,event_json,event_hash,object_id) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![context.owner_id.to_string(), event_seq as i64, event.event_id.to_string(), event.event_type, event_bytes, event.event_hash, object_id],
            )
            .map_err(database_error)?;
        crash_at("after_event_insert");
        transaction
            .execute(
                "INSERT INTO outbox(owner_id,event_seq,acknowledged) VALUES (?1,?2,0)",
                params![context.owner_id.to_string(), event_seq as i64],
            )
            .map_err(database_error)?;
        crash_at("after_outbox_insert");
        let outcome = MutationOutcome {
            event,
            generated: material.generated,
        };
        let outcome_bytes = outcome.canonical_bytes()?;
        transaction
            .execute(
                "UPDATE operations SET outcome=?4 WHERE owner_id=?1 AND client_id=?2 AND operation_id=?3",
                params![context.owner_id.to_string(), context.client_id.to_string(), operation_id, outcome_bytes],
            )
            .map_err(database_error)?;
        crash_at("after_stored_response");
        transaction.commit().map_err(database_error)?;
        drop(active);
        crash_at("after_db_commit_before_projection");
        #[cfg(feature = "semantic-test-failpoints")]
        self.pause_after_commit_if_armed();
        project_and_ack(&self.root, &self.coordinator)?;
        Ok(outcome)
    }

    pub fn ledger_head(&self) -> Result<u64> {
        let connection = open_connection(&self.root)?;
        let head: i64 = connection
            .query_row(
                "SELECT COALESCE(MAX(event_seq),0) FROM events WHERE owner_id=?1",
                [self.marker.owner_id.to_string()],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        u64::try_from(head)
            .map_err(|_| SemanticError::CorruptLedger("negative event sequence".to_owned()))
    }

    pub fn claim_at(
        &self,
        ledger_head: u64,
        world_time: DateTime<Utc>,
    ) -> Result<Option<ClaimView>> {
        // One explicit read transaction covers scope resolution AND
        // plaintext materialization, so a concurrent registry_denied commit
        // cannot land in between (see decrypt_object / read_object).
        let mut raw_connection = open_connection(&self.root)?;
        let transaction = raw_connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database_error)?;
        let connection = &transaction;
        let mut statement = connection
            .prepare("SELECT event_seq,object_id FROM events WHERE owner_id=?1 AND event_type='claim_confirmed' AND event_seq<=?2 ORDER BY event_seq DESC")
            .map_err(database_error)?;
        let rows = statement
            .query_map(
                params![self.marker.owner_id.to_string(), ledger_head as i64],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(database_error)?;
        for row in rows {
            let (event_seq, object_id) = row.map_err(database_error)?;
            let event_seq = u64::try_from(event_seq)
                .map_err(|_| SemanticError::CorruptLedger("negative event sequence".to_owned()))?;
            let object: ConfirmationObject =
                serde_json::from_slice(&decrypt_object(connection, &self.root, &object_id)?)
                    .map_err(serialization_error)?;
            let claim = object.claim;
            if claim.recorded_event_seq != event_seq || claim.status != "confirmed" {
                return Err(SemanticError::CorruptLedger(
                    "claim record does not match confirmation event".to_owned(),
                ));
            }
            let starts = claim.valid_from.is_none_or(|from| from <= world_time);
            let ends = claim.valid_to.is_none_or(|to| world_time < to);
            if starts && ends {
                // Look up the stable entity_id this claim is bound to (Task 2.2).
                let entity_id_text: Option<String> = connection
                    .query_row(
                        "SELECT entity_id FROM claim_status WHERE claim_id=?1",
                        [claim.claim_id.to_string()],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(database_error)?
                    .flatten();
                return Ok(Some(build_claim_view(
                    claim,
                    event_seq,
                    entity_id_text.as_deref(),
                )));
            }
        }
        Ok(None)
    }

    /// Scoped, bitemporal claim query: buckets every confirmed claim in
    /// `(domain, subject, predicate)` into `active`/`future`/`past` as of
    /// `ledger_head` (recorded time) and `world_time` (valid time). A claim
    /// is superseded/retracted "as of" `ledger_head` only when the recorded
    /// transition sequence is at or before that head, so replaying an
    /// earlier head correctly excludes later supersession/retraction.
    pub fn claims_current(
        &self,
        ledger_head: u64,
        world_time: DateTime<Utc>,
        domain: &str,
        subject: &str,
        predicate: &str,
    ) -> Result<CurrentClaims> {
        // See claim_at: one explicit read transaction for both scope
        // resolution and plaintext materialization (TOCTOU-safe deny check).
        let mut raw_connection = open_connection(&self.root)?;
        let transaction = raw_connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database_error)?;
        let connection = &transaction;
        let mut statement = connection
            .prepare(
                "SELECT claim_id, confirmed_event_seq, superseded_by_event_seq, retracted_at_event_seq, entity_id
                 FROM claim_status
                 WHERE domain=?1 AND subject=?2 AND predicate=?3 AND confirmed_event_seq<=?4",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(
                params![domain, subject, predicate, ledger_head as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .map_err(database_error)?;

        let mut result = CurrentClaims::default();
        for row in rows {
            let (claim_id, confirmed_event_seq, superseded_by, retracted_at, entity_id) =
                row.map_err(database_error)?;
            let confirmed_event_seq = u64::try_from(confirmed_event_seq)
                .map_err(|_| SemanticError::CorruptLedger("negative event sequence".to_owned()))?;
            let is_superseded_as_of = superseded_by
                .map(|seq| seq as u64 <= ledger_head)
                .unwrap_or(false);
            let is_retracted_as_of = retracted_at
                .map(|seq| seq as u64 <= ledger_head)
                .unwrap_or(false);

            let object_id: String = connection
                .query_row(
                    "SELECT object_id FROM events WHERE owner_id=?1 AND event_seq=?2",
                    params![self.marker.owner_id.to_string(), confirmed_event_seq as i64],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            let object: ConfirmationObject =
                serde_json::from_slice(&decrypt_object(connection, &self.root, &object_id)?)
                    .map_err(serialization_error)?;
            let claim = object.claim;
            if claim.claim_id.to_string() != claim_id {
                return Err(SemanticError::CorruptLedger(
                    "claim_status row does not match confirmation event".to_owned(),
                ));
            }
            let view = build_claim_view(claim, confirmed_event_seq, entity_id.as_deref());

            if is_superseded_as_of || is_retracted_as_of {
                result.past.push(view);
                continue;
            }
            let started = view.valid_from.is_none_or(|from| from <= world_time);
            let ended = view.valid_to.is_some_and(|to| world_time >= to);
            if ended {
                result.past.push(view);
            } else if started {
                result.active.push(view);
            } else {
                result.future.push(view);
            }
        }
        Ok(result)
    }

    /// Unscoped variant of [`Self::claims_current`]: buckets every confirmed
    /// claim across every `(domain, subject, predicate)` scope into
    /// `active`/`future`/`past` as of `ledger_head`/`world_time`. Used by
    /// external projection adapters (Tantivy/Petgraph/generated Markdown,
    /// Task 2.1) that need to enumerate the whole current claim set rather
    /// than look up one scope at a time.
    pub fn all_claims_current(
        &self,
        ledger_head: u64,
        world_time: DateTime<Utc>,
    ) -> Result<CurrentClaims> {
        // Same TOCTOU-safe single-transaction shape as claims_current: scope
        // resolution and plaintext materialization happen in one explicit
        // read transaction so a concurrent registry_denied commit cannot
        // land in between.
        let mut raw_connection = open_connection(&self.root)?;
        let transaction = raw_connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database_error)?;
        let connection = &transaction;
        let mut statement = connection
            .prepare(
                "SELECT claim_id, confirmed_event_seq, superseded_by_event_seq, retracted_at_event_seq, entity_id
                 FROM claim_status
                 WHERE confirmed_event_seq<=?1",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![ledger_head as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(database_error)?;

        let mut result = CurrentClaims::default();
        for row in rows {
            let (claim_id, confirmed_event_seq, superseded_by, retracted_at, entity_id) =
                row.map_err(database_error)?;
            let confirmed_event_seq = u64::try_from(confirmed_event_seq)
                .map_err(|_| SemanticError::CorruptLedger("negative event sequence".to_owned()))?;
            let is_superseded_as_of = superseded_by
                .map(|seq| seq as u64 <= ledger_head)
                .unwrap_or(false);
            let is_retracted_as_of = retracted_at
                .map(|seq| seq as u64 <= ledger_head)
                .unwrap_or(false);

            let object_id: String = connection
                .query_row(
                    "SELECT object_id FROM events WHERE owner_id=?1 AND event_seq=?2",
                    params![self.marker.owner_id.to_string(), confirmed_event_seq as i64],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            let object: ConfirmationObject =
                serde_json::from_slice(&decrypt_object(connection, &self.root, &object_id)?)
                    .map_err(serialization_error)?;
            let claim = object.claim;
            if claim.claim_id.to_string() != claim_id {
                return Err(SemanticError::CorruptLedger(
                    "claim_status row does not match confirmation event".to_owned(),
                ));
            }
            let view = build_claim_view(claim, confirmed_event_seq, entity_id.as_deref());

            if is_superseded_as_of || is_retracted_as_of {
                result.past.push(view);
                continue;
            }
            let started = view.valid_from.is_none_or(|from| from <= world_time);
            let ended = view.valid_to.is_some_and(|to| world_time >= to);
            if ended {
                result.past.push(view);
            } else if started {
                result.active.push(view);
            } else {
                result.future.push(view);
            }
        }
        Ok(result)
    }

    /// Lists every proposal still awaiting review (Task 5.1 Inbox). Owner
    /// (store) scoped, not client-scoped — any authenticated client can see
    /// the whole inbox regardless of who proposed each item.
    pub fn list_pending_proposals(&self) -> Result<Vec<ProposalSummary>> {
        let mut raw_connection = open_connection(&self.root)?;
        let transaction = raw_connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database_error)?;
        let connection = &transaction;
        let mut statement = connection
            .prepare(
                "SELECT event_seq,event_json,object_id FROM events \
                 WHERE owner_id=?1 AND event_type='claim_proposed' ORDER BY event_seq ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![self.marker.owner_id.to_string()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(database_error)?;
        let mut summaries = Vec::new();
        for row in rows {
            let (event_seq, event_json, object_id) = row.map_err(database_error)?;
            let event_seq = u64::try_from(event_seq)
                .map_err(|_| SemanticError::CorruptLedger("negative event sequence".to_owned()))?;
            let envelope: EventEnvelope =
                serde_json::from_slice(&event_json).map_err(serialization_error)?;
            let proposal: ProposalObject =
                serde_json::from_slice(&decrypt_object(connection, &self.root, &object_id)?)
                    .map_err(serialization_error)?;
            let status: Option<String> = connection
                .query_row(
                    "SELECT status FROM proposal_status WHERE proposal_id=?1",
                    [proposal.proposal_id.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database_error)?;
            if status.as_deref() != Some("proposed") {
                continue;
            }
            summaries.push(ProposalSummary {
                proposal_id: proposal.proposal_id,
                domain: proposal.draft.domain,
                subject: proposal.draft.subject,
                predicate: proposal.draft.predicate,
                value: proposal.draft.value,
                claim_kind: proposal.draft.claim_kind,
                provenance_kind: proposal.provenance.kind().to_owned(),
                submitted_at: envelope.recorded_at,
                event_seq,
            });
        }
        Ok(summaries)
    }

    /// Flat chronological claim history for one `(domain, subject,
    /// predicate)` scope (Task 5.1 Entity timeline) — every confirmed claim
    /// ever recorded in that scope, oldest first, unlike [`Self::claims_current`]
    /// which buckets only the state as-of a given ledger head.
    pub fn claim_timeline(
        &self,
        domain: &str,
        subject: &str,
        predicate: &str,
    ) -> Result<Vec<ClaimView>> {
        let mut raw_connection = open_connection(&self.root)?;
        let transaction = raw_connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database_error)?;
        let connection = &transaction;
        let mut statement = connection
            .prepare(
                "SELECT claim_id, confirmed_event_seq, entity_id
                 FROM claim_status
                 WHERE domain=?1 AND subject=?2 AND predicate=?3
                 ORDER BY confirmed_event_seq ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![domain, subject, predicate], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(database_error)?;

        let mut timeline = Vec::new();
        for row in rows {
            let (claim_id, confirmed_event_seq, entity_id) = row.map_err(database_error)?;
            let confirmed_event_seq = u64::try_from(confirmed_event_seq)
                .map_err(|_| SemanticError::CorruptLedger("negative event sequence".to_owned()))?;
            let object_id: String = connection
                .query_row(
                    "SELECT object_id FROM events WHERE owner_id=?1 AND event_seq=?2",
                    params![self.marker.owner_id.to_string(), confirmed_event_seq as i64],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            let object: ConfirmationObject =
                serde_json::from_slice(&decrypt_object(connection, &self.root, &object_id)?)
                    .map_err(serialization_error)?;
            let claim = object.claim;
            if claim.claim_id.to_string() != claim_id {
                return Err(SemanticError::CorruptLedger(
                    "claim_status row does not match confirmation event".to_owned(),
                ));
            }
            timeline.push(build_claim_view(
                claim,
                confirmed_event_seq,
                entity_id.as_deref(),
            ));
        }
        Ok(timeline)
    }

    /// Human-readable evidence for the Inbox diff view (Task 5.1). Resolves
    /// evidence content straight from the content-addressed object the
    /// proposal's provenance points to — not through the per-client
    /// `operations` idempotency cache, so any reviewer can inspect any
    /// proposal's evidence regardless of who proposed it.
    pub fn evidence_for(&self, proposal_id: Uuid) -> Result<EvidenceSummary> {
        let mut raw_connection = open_connection(&self.root)?;
        let transaction = raw_connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database_error)?;
        let connection = &transaction;
        let (proposal, _event_seq, _submitted_at) =
            resolve_proposal_by_id(connection, &self.root, self.marker.owner_id, proposal_id)?;
        let provenance_kind = proposal.provenance.kind().to_owned();
        let (
            excerpt,
            source_id,
            quote_hash,
            value_located,
            value_offset,
            value_len,
            excerpt_truncated,
            additional_sources,
        ) = match &proposal.provenance {
            Provenance::Evidence {
                source_id,
                object_id,
                byte_start,
                byte_end,
                quote_hash,
                ..
            } => {
                let text =
                    decrypt_text_span(connection, &self.root, object_id, *byte_start, *byte_end)?;
                (
                    Some(text),
                    Some(*source_id),
                    Some(quote_hash.clone()),
                    false,
                    None,
                    None,
                    false,
                    Vec::new(),
                )
            }
            Provenance::UserAssertion {
                utterance_object_id,
                utterance_byte_start,
                utterance_byte_end,
                ..
            } => {
                let text = decrypt_text_span(
                    connection,
                    &self.root,
                    utterance_object_id,
                    *utterance_byte_start,
                    *utterance_byte_end,
                )?;
                (Some(text), None, None, false, None, None, false, Vec::new())
            }
            Provenance::Inference { evidence, .. } => {
                if evidence.is_empty() {
                    (None, None, None, false, None, None, false, Vec::new())
                } else {
                    // Decrypt every span, tracking rendition_id.
                    // Deviation from plan pseudocode: `InferenceEvidenceSpan.rendition_id`
                    // is `Uuid`, not `String`, so we stringify here.
                    let mut decrypted: Vec<(String, String)> = Vec::with_capacity(evidence.len());
                    for span in evidence {
                        let text = decrypt_text_span(
                            connection,
                            &self.root,
                            &span.object_id,
                            span.byte_start,
                            span.byte_end,
                        )?;
                        decrypted.push((span.rendition_id.to_string(), text));
                    }
                    // Pick primary span: first one containing the value; if none, first span.
                    // Deviation from plan pseudocode: the proposal value lives at
                    // `proposal.draft.value` (a `serde_json::Value`, not wrapped in `Option`),
                    // so we pass `Some(&value)` directly rather than `value.as_ref()`.
                    let value = &proposal.draft.value;
                    let candidates = crate::snippet::normalize_value_candidates(value);
                    let primary_idx = decrypted
                        .iter()
                        .position(|(_, text)| {
                            candidates.iter().any(|c| text.find(c.as_str()).is_some())
                        })
                        .unwrap_or(0);
                    let (_primary_rendition, primary_text) = &decrypted[primary_idx];
                    let additional_rendition_ids: Vec<String> = decrypted
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != primary_idx)
                        .map(|(_, (rid, _))| rid.clone())
                        .collect();
                    let snippet = crate::snippet::build_value_snippet(
                        primary_text,
                        Some(value),
                        &additional_rendition_ids,
                    );
                    (
                        Some(snippet.excerpt),
                        None,
                        None,
                        snippet.value_located,
                        snippet.value_offset,
                        snippet.value_len,
                        snippet.excerpt_truncated,
                        snippet.additional_sources,
                    )
                }
            }
            Provenance::Mechanical { output_hash, .. } => (
                None,
                None,
                Some(output_hash.clone()),
                false,
                None,
                None,
                false,
                Vec::new(),
            ),
        };
        Ok(EvidenceSummary {
            provenance_kind,
            excerpt,
            source_id,
            quote_hash,
            value_located,
            value_offset,
            value_len,
            excerpt_truncated,
            additional_sources,
        })
    }

    /// The schema version this store was created with (currently always 1 —
    /// no migration path exists yet). Exposed for callers that need to
    /// reproduce the `ledger_head:purge_epoch:schema_version` composite
    /// checksum identity outside the purge saga (e.g. projection adapters).
    pub fn schema_version(&self) -> u8 {
        self.marker.schema_version
    }

    pub fn recover(&self, _request: ManualRecovery) -> Result<()> {
        #[cfg(feature = "semantic-test-failpoints")]
        self.coordinator
            .recovery_waiting
            .fetch_add(1, Ordering::SeqCst);
        let _maintenance = self.coordinator.maintenance.write();
        #[cfg(feature = "semantic-test-failpoints")]
        self.coordinator
            .recovery_waiting
            .fetch_sub(1, Ordering::SeqCst);
        let mut connection = open_connection(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let active = ActiveTransaction::new(&self.coordinator);
        validate_ledger(&transaction)?;
        remove_staging_files(&self.root)?;
        remove_unreferenced_objects(&self.root, &transaction)?;
        transaction.commit().map_err(database_error)?;
        drop(active);
        let _projection = self.coordinator.projection.lock();
        project_and_ack_without_guard(&self.root, &self.coordinator)?;
        Ok(())
    }

    pub fn diagnostics(&self) -> Result<StoreDiagnostics> {
        diagnostics(&self.root)
    }

    /// The owner's current key-encryption-key epoch. New objects are
    /// wrapped under this epoch; older objects may still be wrapped under
    /// earlier epochs until a rotation rewraps them.
    pub fn current_epoch(&self) -> Result<u64> {
        let connection = open_connection(&self.root)?;
        let epoch: i64 = connection
            .query_row("SELECT MAX(epoch) FROM epoch_keys", [], |row| row.get(0))
            .map_err(database_error)?;
        u64::try_from(epoch).map_err(|_| SemanticError::CorruptLedger("invalid epoch".to_owned()))
    }

    pub fn wrapped_key_count(&self) -> Result<usize> {
        let connection = open_connection(&self.root)?;
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM wrapped_keys", [], |row| row.get(0))
            .map_err(database_error)?;
        usize::try_from(count)
            .map_err(|_| SemanticError::CorruptLedger("invalid wrapped key count".to_owned()))
    }

    /// Cryptographically erases a single object: deletes its wrapped DEK so
    /// the ciphertext bytes on disk (which may still exist) can never be
    /// decrypted again. This is the per-object primitive a future hard-purge
    /// saga calls for each target; it does not touch the event ledger, so
    /// object identity/history is untouched — only recoverability is
    /// destroyed.
    pub fn destroy_wrapped_key(&self, object_id: &str) -> Result<()> {
        let _maintenance = self.coordinator.maintenance.read();
        let _writer = self.coordinator.writer.lock();
        let connection = open_connection(&self.root)?;
        connection
            .execute("DELETE FROM wrapped_keys WHERE object_id=?1", [object_id])
            .map_err(database_error)?;
        Ok(())
    }

    /// Generates a new epoch KEK, rewraps every existing object's DEK under
    /// it, and destroys the previous epoch's KEK. Requires the exclusive
    /// maintenance lock so no concurrent mutation can create a new
    /// wrapped-key row under the old epoch mid-rotation. This is the
    /// defense-in-depth half of a hard purge (ADR Decision 7): even if an
    /// old epoch KEK were ever compromised, it no longer protects anything
    /// once every surviving object has been rewrapped under a fresh one.
    pub fn rotate_epoch_and_rewrap(&self) -> Result<u64> {
        let _maintenance = self.coordinator.maintenance.write();
        let mut connection = open_connection(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let active = ActiveTransaction::new(&self.coordinator);

        let old_epoch: i64 = transaction
            .query_row("SELECT MAX(epoch) FROM epoch_keys", [], |row| row.get(0))
            .map_err(database_error)?;
        let old_kek = load_epoch_key(&transaction, old_epoch)?;
        let new_epoch = old_epoch + 1;
        let new_key = Aes256Gcm::generate_key(&mut OsRng);
        transaction
            .execute(
                "INSERT INTO epoch_keys(epoch,key_material,created_at) VALUES (?1,?2,?3)",
                params![new_epoch, new_key.as_slice(), self.clock.now().to_rfc3339()],
            )
            .map_err(database_error)?;
        let new_kek = Aes256Gcm::new(&new_key);

        let rows: Vec<(String, Vec<u8>, Vec<u8>)> = {
            let mut statement = transaction
                .prepare(
                    "SELECT object_id, wrapped_dek, dek_nonce FROM wrapped_keys WHERE epoch=?1",
                )
                .map_err(database_error)?;
            statement
                .query_map([old_epoch], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .map_err(database_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(database_error)?
        };
        for (object_id, wrapped_dek, dek_nonce) in rows {
            let wrap_nonce = Nonce::from_slice(&dek_nonce);
            let dek_bytes = old_kek
                .decrypt(wrap_nonce, wrapped_dek.as_ref())
                .map_err(|_| SemanticError::ObjectUnavailable(object_id.clone()))?;
            let new_wrap_nonce = Aes256Gcm::generate_nonce(&mut OsRng);
            let new_wrapped = new_kek
                .encrypt(&new_wrap_nonce, dek_bytes.as_ref())
                .map_err(|_| SemanticError::Serialization("key rewrap failed".to_owned()))?;
            transaction
                .execute(
                    "UPDATE wrapped_keys SET epoch=?2, wrapped_dek=?3, dek_nonce=?4 WHERE object_id=?1",
                    params![object_id, new_epoch, new_wrapped, new_wrap_nonce.as_slice()],
                )
                .map_err(database_error)?;
        }
        transaction
            .execute("DELETE FROM epoch_keys WHERE epoch=?1", [old_epoch])
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
        drop(active);
        u64::try_from(new_epoch)
            .map_err(|_| SemanticError::CorruptLedger("invalid epoch".to_owned()))
    }

    pub fn storage_pragmas(&self) -> Result<StoragePragmas> {
        let connection = open_connection(&self.root)?;
        Ok(StoragePragmas {
            journal_mode: connection
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .map_err(database_error)?,
            synchronous: connection
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .map_err(database_error)?,
            foreign_keys: connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                .map_err(database_error)?,
            busy_timeout_ms: connection
                .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
                .map_err(database_error)?,
        })
    }

    #[cfg(feature = "semantic-test-failpoints")]
    pub fn pause_after_commit_for_test(&self) -> SemanticTestPause {
        self.pause_after_commit.flags.lock().armed = true;
        SemanticTestPause {
            state: Arc::clone(&self.pause_after_commit),
        }
    }

    /// Arms a pause inside `decrypt_object`, right after its deny/seal check
    /// and before it materializes plaintext, so a test can race a
    /// concurrent `append_registry_denial` against an in-flight read and
    /// prove the read observes one consistent snapshot across every object
    /// it touches (e.g. every row of one `claims_current` call) rather than
    /// a torn mix of pre- and post-denial state.
    #[cfg(feature = "semantic-test-failpoints")]
    pub fn pause_read_after_deny_check_for_test(&self) -> SemanticTestPause {
        self.coordinator
            .pause_read_after_deny_check
            .flags
            .lock()
            .armed = true;
        SemanticTestPause {
            state: Arc::clone(&self.coordinator.pause_read_after_deny_check),
        }
    }

    #[cfg(feature = "semantic-test-failpoints")]
    fn pause_after_commit_if_armed(&self) {
        let mut flags = self.pause_after_commit.flags.lock();
        if !flags.armed {
            return;
        }
        flags.entered = true;
        self.pause_after_commit.condvar.notify_all();
        while !flags.released {
            self.pause_after_commit.condvar.wait(&mut flags);
        }
    }

    #[cfg(feature = "semantic-test-failpoints")]
    pub fn recovery_blocked_for_test(&self) -> bool {
        self.coordinator.recovery_waiting.load(Ordering::SeqCst) > 0
    }

    #[cfg(feature = "semantic-test-failpoints")]
    pub fn begin_test_transaction(&self) -> Result<SemanticTestTransaction> {
        let connection = open_connection(&self.root)?;
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(database_error)?;
        self.coordinator
            .active_transactions
            .fetch_add(1, Ordering::SeqCst);
        Ok(SemanticTestTransaction {
            connection,
            coordinator: Arc::clone(&self.coordinator),
        })
    }

    pub fn projection_state(&self) -> Result<ProjectionState> {
        let bytes = fs::read(self.root.join(PROJECTION_FILE)).map_err(io_error)?;
        serde_json::from_slice(&bytes).map_err(serialization_error)
    }

    pub fn object_exists(&self, object_id: &str) -> bool {
        object_path(&self.root, object_id).is_ok_and(|path| path.is_file())
    }

    pub fn object_json(&self, object_id: &str) -> Result<Value> {
        serde_json::from_slice(&read_object(&self.root, object_id)?).map_err(serialization_error)
    }

    /// Current local purge-registry epoch; 0 means no denial has ever been
    /// appended (or applied via sync) to this copy.
    pub fn registry_epoch(&self) -> Result<u64> {
        let connection = open_connection(&self.root)?;
        Ok(local_registry_head(&connection)?
            .map(|entry| entry.epoch)
            .unwrap_or(0))
    }

    /// True if `id` (an object ID or claim ID) is currently denied.
    pub fn is_denied(&self, id: &str) -> Result<bool> {
        let connection = open_connection(&self.root)?;
        let denied: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM purge_denied_ids WHERE denied_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error)?;
        Ok(denied.is_some())
    }

    /// True if the store is currently sealed: every plaintext-returning read
    /// (`decrypt_object`/`read_object` and everything built on them) fails
    /// closed with `SemanticError::RegistrySealed` until `sync_purge_registry`
    /// succeeds.
    pub fn is_registry_sealed(&self) -> Result<bool> {
        let connection = open_connection(&self.root)?;
        let value: String = connection
            .query_row(
                "SELECT value FROM meta WHERE key='registry_sealed'",
                [],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        Ok(value == "true")
    }

    /// Appends one purge-registry denial entry for `ids`, replicates it to
    /// the configured targets, and returns the new epoch. Raw primitive: not
    /// capability-gated, not idempotent-by-operation-id, and not nonce-bound
    /// -- the hard-purge saga wires those guarantees around this call.
    /// Requires at least 2 configured replication targets. The local
    /// commit -- which alone already protects this store's own reads via
    /// `decrypt_object`'s deny check -- is never rolled back by a
    /// replication shortfall: deny-first means a crash or unreachable
    /// target after the local commit must not un-deny anything. On quorum
    /// shortfall this returns `RegistryQuorumFailed` so a caller (the saga)
    /// can retry replication alone without re-denying.
    pub fn append_registry_denial(&self, ids: &[String]) -> Result<u64> {
        if ids.is_empty() {
            return Err(SemanticError::InvalidClaim(
                "purge registry denial requires at least one ID".to_owned(),
            ));
        }
        if self.purge_registry_targets.len() < 2 {
            return Err(SemanticError::InvalidRoot(
                "purge registry requires at least 2 configured replication targets".to_owned(),
            ));
        }
        let mut denied_ids: Vec<String> = ids.to_vec();
        denied_ids.sort();
        denied_ids.dedup();

        let _maintenance = self.coordinator.maintenance.read();
        let _writer = self.coordinator.writer.lock();
        let mut connection = open_connection(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let head = local_registry_head(&transaction)?;
        // Idempotent retry: if the current head already denies exactly this
        // normalized batch, this call is a repeat of the immediately
        // preceding one (e.g. a purge saga step retrying replication after a
        // quorum shortfall) -- reuse that entry instead of minting a new
        // epoch for the same batch. A batch that differs even partially
        // from the head (a distinct call, interleaved with another) still
        // gets its own fresh epoch below.
        let entry = if head
            .as_ref()
            .is_some_and(|entry| entry.denied_ids == denied_ids)
        {
            head.expect("checked Some above")
        } else {
            let next_epoch = head.as_ref().map_or(1, |entry| entry.epoch + 1);
            let prior_hash = head.map(|entry| entry.entry_hash);
            let entry_hash = registry_entry_hash(next_epoch, &denied_ids, &prior_hash)?;
            let fresh = RegistryEntryFile {
                epoch: next_epoch,
                denied_ids,
                entry_hash,
                prior_entry_hash: prior_hash,
            };
            insert_registry_entry(&transaction, &fresh)?;
            fresh
        };
        transaction.commit().map_err(database_error)?;

        let quorum = quorum_needed(self.purge_registry_targets.len());
        let mut acknowledged = 0usize;
        for target in &self.purge_registry_targets {
            if fs::create_dir_all(target).is_ok()
                && write_registry_target_entry(target, &entry).is_ok()
            {
                acknowledged += 1;
            }
        }
        if acknowledged < quorum {
            return Err(SemanticError::RegistryQuorumFailed(format!(
                "only {acknowledged} of {quorum} required replication targets acknowledged epoch {}",
                entry.epoch
            )));
        }
        Ok(entry.epoch)
    }

    /// Re-checks the configured replication targets and, if a reachable
    /// quorum reports epochs this copy is missing, applies them (verifying
    /// hash-chain continuity) before unsealing. Fails closed (returns an
    /// error, store remains sealed) if quorum cannot be reached or a
    /// fetched entry does not chain from the current local head.
    pub fn sync_purge_registry(&self) -> Result<()> {
        let _maintenance = self.coordinator.maintenance.read();
        let _writer = self.coordinator.writer.lock();
        let mut connection = open_connection(&self.root)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;

        if !self.purge_registry_targets.is_empty() {
            let quorum = quorum_needed(self.purge_registry_targets.len());
            let target_heads: Vec<RegistryEntryFile> = self
                .purge_registry_targets
                .iter()
                .filter_map(|target| registry_target_head(target))
                .collect();
            if target_heads.len() < quorum {
                set_registry_sealed(&transaction, true)?;
                transaction.commit().map_err(database_error)?;
                return Err(SemanticError::RegistryQuorumFailed(format!(
                    "only {} of {quorum} required targets reachable",
                    target_heads.len()
                )));
            }
            let target_max = target_heads
                .iter()
                .map(|entry| entry.epoch)
                .max()
                .unwrap_or(0);
            let mut local_head = local_registry_head(&transaction)?;
            let mut next_epoch = local_head.as_ref().map_or(1, |entry| entry.epoch + 1);
            while next_epoch <= target_max {
                let Some(entry) = self
                    .purge_registry_targets
                    .iter()
                    .find_map(|target| read_registry_target_entry(target, next_epoch))
                else {
                    set_registry_sealed(&transaction, true)?;
                    transaction.commit().map_err(database_error)?;
                    return Err(SemanticError::RegistryQuorumFailed(format!(
                        "no reachable target has purge epoch {next_epoch}"
                    )));
                };
                let prior_hash = local_head
                    .as_ref()
                    .map(|current| current.entry_hash.clone());
                let expected_hash =
                    registry_entry_hash(entry.epoch, &entry.denied_ids, &prior_hash)?;
                if entry.entry_hash != expected_hash || entry.prior_entry_hash != prior_hash {
                    set_registry_sealed(&transaction, true)?;
                    transaction.commit().map_err(database_error)?;
                    return Err(SemanticError::CorruptLedger(format!(
                        "purge registry hash chain broken at epoch {next_epoch}"
                    )));
                }
                insert_registry_entry(&transaction, &entry)?;
                local_head = Some(entry.clone());
                next_epoch += 1;
            }
            if evaluate_registry_seal(&transaction, &self.purge_registry_targets)? {
                set_registry_sealed(&transaction, true)?;
                transaction.commit().map_err(database_error)?;
                return Err(SemanticError::RegistryQuorumFailed(
                    "registry still disagrees with a quorum of targets after sync".to_owned(),
                ));
            }
        }
        set_registry_sealed(&transaction, false)?;
        transaction.commit().map_err(database_error)?;
        Ok(())
    }

    /// Previews a hard purge of `targets` (object IDs), returning a
    /// `preview_hash` the caller must echo back to `purge_execute` and a
    /// single-use nonce bound to that hash, expiring in 60 seconds (ADR
    /// Decision 7). Claim-ID-to-object-ID resolution stays the caller's
    /// job in this phase, matching Task 1.2's documented context-dimension
    /// simplification -- both are deferred scope, not silently dropped.
    pub fn purge_preview(&self, targets: &[String]) -> Result<PurgePreview> {
        if targets.is_empty() {
            return Err(SemanticError::InvalidClaim(
                "purge preview requires at least one target".to_owned(),
            ));
        }
        let mut sorted = targets.to_vec();
        sorted.sort();
        sorted.dedup();
        // Reject a malformed target now rather than letting it enter a
        // running saga: object_path()'s format check would otherwise only
        // fire inside live_deleted/projections_cleaned, after registry_denied
        // and key_revoked already committed for the *other*, well-formed
        // targets in the same batch -- permanently stalling the saga instead
        // of failing before anything irreversible happens.
        for target in &sorted {
            object_path(&self.root, target)?;
        }
        let preview_hash = sha256(&canonical_bytes(&sorted)?);
        let nonce = Uuid::now_v7().to_string();
        let expires_at = self.clock.now() + chrono::Duration::seconds(60);
        let targets_json = serde_json::to_string(&sorted).map_err(serialization_error)?;

        let _maintenance = self.coordinator.maintenance.read();
        let _writer = self.coordinator.writer.lock();
        let connection = open_connection(&self.root)?;
        connection
            .execute(
                "INSERT INTO purge_nonces(nonce,preview_hash,targets_json,expires_at,used_at) VALUES (?1,?2,?3,?4,NULL)",
                params![nonce, preview_hash, targets_json, expires_at.to_rfc3339()],
            )
            .map_err(database_error)?;

        Ok(PurgePreview {
            preview_hash,
            nonce,
            expires_at,
            targets: sorted,
        })
    }

    /// Authorizes and starts (or idempotently resumes/replays) a hard-purge
    /// saga bound to a `purge_preview` result. Requires the `purge`
    /// capability. Same `(client_id, operation_id)` with the same
    /// `preview_hash` resumes/replays; a different `preview_hash` under the
    /// same `operation_id` is `IdempotencyConflict`. The nonce is validated
    /// and consumed only when a saga is first created -- resuming an
    /// already-started saga (including after a crash) needs no nonce, since
    /// authorization already happened at creation.
    pub fn purge_execute(
        &self,
        context: &TrustedContext,
        operation_id: &str,
        preview_hash: &str,
        nonce: &str,
    ) -> Result<PurgeReceipt> {
        validate_context(&self.marker, context)?;
        validate_operation_id(operation_id)?;
        let request_hash = sha256(preview_hash.as_bytes());

        let purge_id = {
            let _maintenance = self.coordinator.maintenance.read();
            let _writer = self.coordinator.writer.lock();
            let mut connection = open_connection(&self.root)?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database_error)?;

            let registered: Option<i64> = transaction
                .query_row(
                    "SELECT 1 FROM clients WHERE client_id=?1",
                    [context.client_id.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database_error)?;
            if registered.is_none() {
                return Err(SemanticError::MissingDependency(
                    "client is not registered for this store".to_owned(),
                ));
            }
            let has_capability: Option<i64> = transaction
                .query_row(
                    "SELECT 1 FROM client_capabilities WHERE client_id=?1 AND capability='purge'",
                    [context.client_id.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database_error)?;
            if has_capability.is_none() {
                return Err(SemanticError::CapabilityDenied(
                    "client lacks required capability: purge".to_owned(),
                ));
            }

            let existing: Option<(String, String)> = transaction
                .query_row(
                    "SELECT purge_id, request_hash FROM purge_sagas WHERE client_id=?1 AND operation_id=?2",
                    params![context.client_id.to_string(), operation_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(database_error)?;

            if let Some((purge_id, stored_request_hash)) = existing {
                if stored_request_hash != request_hash {
                    return Err(SemanticError::IdempotencyConflict);
                }
                transaction.commit().map_err(database_error)?;
                Uuid::parse_str(&purge_id)
                    .map_err(|_| SemanticError::CorruptLedger("invalid purge_id".to_owned()))?
            } else {
                let nonce_row: Option<(String, String, String, Option<String>)> = transaction
                    .query_row(
                        "SELECT preview_hash, targets_json, expires_at, used_at FROM purge_nonces WHERE nonce=?1",
                        [nonce],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    )
                    .optional()
                    .map_err(database_error)?;
                let Some((stored_preview_hash, targets_json, expires_at, used_at)) = nonce_row
                else {
                    return Err(SemanticError::InvalidClaim(
                        "unknown purge nonce".to_owned(),
                    ));
                };
                if used_at.is_some() {
                    return Err(SemanticError::InvalidClaim(
                        "purge nonce already used".to_owned(),
                    ));
                }
                if stored_preview_hash != preview_hash {
                    return Err(SemanticError::InvalidClaim(
                        "preview_hash does not match nonce".to_owned(),
                    ));
                }
                let expires_at: DateTime<Utc> = expires_at
                    .parse()
                    .map_err(|_| SemanticError::CorruptLedger("invalid nonce expiry".to_owned()))?;
                if self.clock.now() >= expires_at {
                    return Err(SemanticError::InvalidClaim(
                        "purge nonce expired".to_owned(),
                    ));
                }
                transaction
                    .execute(
                        "UPDATE purge_nonces SET used_at=?1 WHERE nonce=?2",
                        params![self.clock.now().to_rfc3339(), nonce],
                    )
                    .map_err(database_error)?;

                let purge_id = Uuid::now_v7();
                transaction
                    .execute(
                        "INSERT INTO purge_sagas(purge_id,client_id,operation_id,request_hash,targets_json,preview_hash,state,created_at) VALUES (?1,?2,?3,?4,?5,?6,'requested',?7)",
                        params![
                            purge_id.to_string(),
                            context.client_id.to_string(),
                            operation_id,
                            request_hash,
                            targets_json,
                            preview_hash,
                            self.clock.now().to_rfc3339()
                        ],
                    )
                    .map_err(database_error)?;
                transaction.commit().map_err(database_error)?;
                crash_at("purge_after_requested");
                purge_id
            }
        };

        self.run_purge_saga(purge_id)
    }

    /// Continues an in-flight hard-purge saga from its persisted state.
    /// Needs no capability/nonce: authorization already happened when the
    /// saga was created by `purge_execute`. Used to recover after a crash,
    /// or to retry a step that previously failed (e.g. a registry
    /// replication quorum shortfall).
    pub fn purge_resume(&self, purge_id: Uuid) -> Result<PurgeReceipt> {
        self.run_purge_saga(purge_id)
    }

    /// Current receipt/state for a purge saga, without advancing it.
    pub fn purge_status(&self, purge_id: Uuid) -> Result<PurgeReceipt> {
        self.purge_receipt(purge_id)
    }

    fn run_purge_saga(&self, purge_id: Uuid) -> Result<PurgeReceipt> {
        loop {
            let saga = self.load_purge_saga(purge_id)?;
            match saga.state.as_str() {
                "requested" => {
                    self.advance_purge_registry_denied(purge_id, &saga.targets)?;
                    crash_at("purge_after_registry_denied");
                }
                "registry_denied" => {
                    self.advance_purge_key_revoked(purge_id, &saga.targets)?;
                    crash_at("purge_after_key_revoked");
                }
                "key_revoked" => {
                    self.advance_purge_live_deleted(purge_id, &saga.targets)?;
                    crash_at("purge_after_live_deleted");
                }
                "live_deleted" => {
                    self.advance_purge_projections_cleaned(purge_id, &saga.targets)?;
                    crash_at("purge_after_projections_cleaned");
                }
                "projections_cleaned" => {
                    self.advance_purge_retention_pending(purge_id, &saga.targets)?;
                    crash_at("purge_after_retention_pending");
                }
                "retention_pending" => {
                    self.advance_purge_completed(purge_id)?;
                    crash_at("purge_after_completed");
                }
                "completed" => return self.purge_receipt(purge_id),
                other => {
                    return Err(SemanticError::CorruptLedger(format!(
                        "unknown purge saga state: {other}"
                    )));
                }
            }
        }
    }

    fn load_purge_saga(&self, purge_id: Uuid) -> Result<PurgeSagaRow> {
        let connection = open_connection(&self.root)?;
        let row: (String, String, Option<i64>, Option<String>, Option<String>) = connection
            .query_row(
                "SELECT state, targets_json, registry_epoch, new_backup_path, composite_checksum FROM purge_sagas WHERE purge_id=?1",
                [purge_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .map_err(database_error)?;
        let (state, targets_json, registry_epoch, new_backup_path, composite_checksum) = row;
        let targets: Vec<String> =
            serde_json::from_str(&targets_json).map_err(serialization_error)?;
        Ok(PurgeSagaRow {
            state,
            targets,
            registry_epoch: registry_epoch.map(|value| value as u64),
            new_backup_path,
            composite_checksum,
        })
    }

    fn set_purge_state(&self, purge_id: Uuid, state: &str) -> Result<()> {
        let connection = open_connection(&self.root)?;
        connection
            .execute(
                "UPDATE purge_sagas SET state=?1 WHERE purge_id=?2",
                params![state, purge_id.to_string()],
            )
            .map_err(database_error)?;
        Ok(())
    }

    fn advance_purge_registry_denied(&self, purge_id: Uuid, targets: &[String]) -> Result<()> {
        let epoch = self.append_registry_denial(targets)?;
        let connection = open_connection(&self.root)?;
        connection
            .execute(
                "UPDATE purge_sagas SET state='registry_denied', registry_epoch=?1 WHERE purge_id=?2",
                params![epoch as i64, purge_id.to_string()],
            )
            .map_err(database_error)?;
        Ok(())
    }

    fn advance_purge_key_revoked(&self, purge_id: Uuid, targets: &[String]) -> Result<()> {
        for target in targets {
            self.destroy_wrapped_key(target)?;
        }
        self.rotate_epoch_and_rewrap()?;
        self.set_purge_state(purge_id, "key_revoked")
    }

    fn advance_purge_live_deleted(&self, purge_id: Uuid, targets: &[String]) -> Result<()> {
        for target in targets {
            let path = object_path(&self.root, target)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(error)),
            }
        }
        self.set_purge_state(purge_id, "live_deleted")
    }

    /// Phase 1 has no wired projections (Tantivy/Petgraph/generated
    /// Markdown land in Phase 2 Task 2.1), so there is nothing to rewrite
    /// or rebuild yet -- but this step still performs a real absence proof
    /// rather than a no-op: every target's on-disk ciphertext, wrapped
    /// key, and denial record are independently re-checked. A stray
    /// leftover copy (e.g. from a crash before `live_deleted` completed,
    /// or a hand-restored file) fails this step closed instead of silently
    /// advancing past it.
    fn advance_purge_projections_cleaned(&self, purge_id: Uuid, targets: &[String]) -> Result<()> {
        let connection = open_connection(&self.root)?;
        for target in targets {
            let path = object_path(&self.root, target)?;
            if path.exists() {
                return Err(SemanticError::CorruptLedger(format!(
                    "absence proof failed: object file for {target} still exists"
                )));
            }
            let has_key: Option<i64> = connection
                .query_row(
                    "SELECT 1 FROM wrapped_keys WHERE object_id=?1",
                    [target],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database_error)?;
            if has_key.is_some() {
                return Err(SemanticError::CorruptLedger(format!(
                    "absence proof failed: wrapped key for {target} still exists"
                )));
            }
            let denied: Option<i64> = connection
                .query_row(
                    "SELECT 1 FROM purge_denied_ids WHERE denied_id=?1",
                    [target],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database_error)?;
            if denied.is_none() {
                return Err(SemanticError::CorruptLedger(format!(
                    "absence proof failed: {target} is not recorded in the purge registry"
                )));
            }
        }
        drop(connection);
        self.set_purge_state(purge_id, "projections_cleaned")
    }

    /// Invalidates (deletes) every backup this store made whose OWN
    /// `wrapped_keys` table still has a live row for at least one purge
    /// target -- i.e. every backup actually capable of decrypting a
    /// target, not every backup this store has ever made -- then creates
    /// and independently verifies one fresh backup, taken after
    /// key_revoked and live_deleted, so it naturally carries neither the
    /// destroyed key nor the deleted object. Only backups this store
    /// itself created and tracks are addressed; an externally copied
    /// backup is an operator responsibility, matching the ADR's own
    /// backup model.
    fn advance_purge_retention_pending(&self, purge_id: Uuid, targets: &[String]) -> Result<()> {
        let paths: Vec<String> = {
            let connection = open_connection(&self.root)?;
            let mut statement = connection
                .prepare("SELECT backup_path FROM purge_backup_sets WHERE invalidated_at IS NULL")
                .map_err(database_error)?;
            statement
                .query_map([], |row| row.get(0))
                .map_err(database_error)?
                .collect::<std::result::Result<Vec<String>, _>>()
                .map_err(database_error)?
        };
        for path in &paths {
            let path_buf = PathBuf::from(path);
            let backup_database = path_buf.join(DATABASE_FILE);
            // Only a backup whose OWN wrapped_keys table still has a row for
            // at least one purge target could actually decrypt it -- a
            // backup taken before the target ever existed, or of unrelated
            // data, must not be destroyed by this purge (ADR Decision 7:
            // "invalidate backups capable of decryption", not every backup).
            let mut can_decrypt_a_target = false;
            if backup_database.is_file() {
                let backup_connection =
                    Connection::open(&backup_database).map_err(database_error)?;
                for target in targets {
                    let has_key: Option<i64> = backup_connection
                        .query_row(
                            "SELECT 1 FROM wrapped_keys WHERE object_id=?1",
                            [target],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(database_error)?;
                    if has_key.is_some() {
                        can_decrypt_a_target = true;
                        break;
                    }
                }
            }
            if !can_decrypt_a_target {
                continue;
            }
            if path_buf.exists() {
                fs::remove_dir_all(&path_buf).map_err(io_error)?;
            }
            let connection = open_connection(&self.root)?;
            connection
                .execute(
                    "UPDATE purge_backup_sets SET invalidated_at=?1 WHERE backup_path=?2",
                    params![self.clock.now().to_rfc3339(), path],
                )
                .map_err(database_error)?;
        }

        let backup_path =
            PathBuf::from(&self.marker.allowed_parent).join(format!("purge-backup-{purge_id}"));
        self.backup_consistent(&backup_path)?;
        let verify_config = SemanticConfig::enabled_for(&self.marker.allowed_parent);
        let verify_store = SemanticStore::open(&backup_path, verify_config)?;
        drop(verify_store);

        let connection = open_connection(&self.root)?;
        connection
            .execute(
                "UPDATE purge_sagas SET state='retention_pending', new_backup_path=?1 WHERE purge_id=?2",
                params![backup_path.to_string_lossy(), purge_id.to_string()],
            )
            .map_err(database_error)?;
        Ok(())
    }

    fn advance_purge_completed(&self, purge_id: Uuid) -> Result<()> {
        let checksum = self.composite_checksum()?;
        let connection = open_connection(&self.root)?;
        connection
            .execute(
                "UPDATE purge_sagas SET state='completed', composite_checksum=?1, completed_at=?2 WHERE purge_id=?3",
                params![checksum, self.clock.now().to_rfc3339(), purge_id.to_string()],
            )
            .map_err(database_error)?;
        Ok(())
    }

    fn composite_checksum(&self) -> Result<String> {
        let connection = open_connection(&self.root)?;
        let ledger_head: i64 = connection
            .query_row(
                "SELECT COALESCE(MAX(event_seq),0) FROM events WHERE owner_id=?1",
                [self.marker.owner_id.to_string()],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        let purge_epoch = local_registry_head(&connection)?.map_or(0, |entry| entry.epoch);
        Ok(sha256(
            format!("{ledger_head}:{purge_epoch}:{}", self.marker.schema_version).as_bytes(),
        ))
    }

    fn purge_receipt(&self, purge_id: Uuid) -> Result<PurgeReceipt> {
        let saga = self.load_purge_saga(purge_id)?;
        Ok(PurgeReceipt {
            purge_id,
            state: saga.state,
            registry_epoch: saga.registry_epoch,
            new_backup_path: saga.new_backup_path,
            composite_checksum: saga.composite_checksum,
        })
    }

    // ── Phase E Task E3.1: trust + operations producers ──────────────────────
    //
    // These methods return the contract types defined in `crate::trust`
    // (TrustFlag, RetrievalTrace, JobSummary, BackupHealth, ClientActivity,
    // EvalSummary). They are the producers that turn the previously-dead-code
    // contract types into live surfaces the Console API (E3.2) will expose.

    /// Detect contradiction flags: groups of active confirmed claims sharing
    /// the same `(domain, subject, predicate)` scope but carrying DISTINCT
    /// values. "Distinct" is deep `serde_json::Value` equality — `[1]` vs `1`
    /// vs `"1"` are three distinct values. Idempotent + deterministic: each
    /// flag's `claim_ids` are sorted, and the resulting flag list is sorted by
    /// the first claim_id in each group.
    pub fn contradictions(&self, head: u64, now: DateTime<Utc>) -> Result<Vec<TrustFlag>> {
        let current = self.all_claims_current(head, now)?;
        // Group active confirmed claims by their semantic scope.
        let mut groups: HashMap<(String, String, String), Vec<&ClaimView>> = HashMap::new();
        for claim in &current.active {
            if claim.status != "confirmed" {
                continue;
            }
            groups
                .entry((
                    claim.domain.clone(),
                    claim.subject.clone(),
                    claim.predicate.clone(),
                ))
                .or_default()
                .push(claim);
        }
        let mut flags = Vec::new();
        for (_, members) in groups {
            if members.len() < 2 {
                continue;
            }
            // Collect the set of distinct values seen in this scope. Uses deep
            // serde_json equality so structural differences count.
            let mut distinct_values: Vec<&Value> = Vec::new();
            for member in &members {
                if !distinct_values.contains(&&member.value) {
                    distinct_values.push(&member.value);
                }
            }
            if distinct_values.len() < 2 {
                continue;
            }
            let mut claim_ids: Vec<String> = members
                .iter()
                .map(|claim| claim.claim_id.to_string())
                .collect();
            claim_ids.sort();
            flags.push(TrustFlag::Contradiction { claim_ids });
        }
        // Stable output ordering: sort by the first claim_id in each flag.
        flags.sort_by(|a, b| {
            let key = |flag: &TrustFlag| match flag {
                TrustFlag::Contradiction { claim_ids } => {
                    claim_ids.first().cloned().unwrap_or_default()
                }
                _ => String::new(),
            };
            key(a).cmp(&key(b))
        });
        Ok(flags)
    }

    /// Detect stale flags: active confirmed claims whose recorded `recorded_at`
    /// timestamp on their confirm event is older than `threshold_days`. The
    /// timestamp is read from the `events` table's `event_json` BLOB (the full
    /// `EventEnvelope.recorded_at` of the `claim_confirmed` event pointed at by
    /// `ClaimView.confirmed_event_seq`) — the only authoritative source of
    /// transaction-time truth in the ledger.
    ///
    /// Staleness uses the active claim's own `confirmed_event_seq` recorded_at
    /// as the authoritative timestamp. Superseded/retracted claims move to
    /// `past` and are not flagged; the surviving active claim's confirm time is
    /// what's measured.
    pub fn staleness(
        &self,
        head: u64,
        threshold_days: u32,
        now: DateTime<Utc>,
    ) -> Result<Vec<TrustFlag>> {
        let current = self.all_claims_current(head, now)?;
        if current.active.is_empty() {
            return Ok(Vec::new());
        }
        // One read transaction for the whole timestamp lookup pass — same
        // TOCTOU-safe shape as all_claims_current.
        let mut raw_connection = open_connection(&self.root)?;
        let transaction = raw_connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database_error)?;
        let connection = &transaction;
        let mut statement = connection
            .prepare(
                "SELECT event_json FROM events \
                 WHERE owner_id=?1 AND event_seq=?2",
            )
            .map_err(database_error)?;
        let mut flags = Vec::new();
        for claim in &current.active {
            if claim.status != "confirmed" {
                continue;
            }
            // Decode the confirm event's recorded_at from its event_json BLOB.
            // claim.confirmed_event_seq is the authoritative "this claim's last
            // audit touch" pointer; supersede/retract point at later sequences
            // but apply to a DIFFERENT active claim, so the active view's own
            // confirmed_event_seq is what matters here.
            let event_bytes: Option<Vec<u8>> = statement
                .query_row(
                    params![
                        self.marker.owner_id.to_string(),
                        claim.confirmed_event_seq as i64
                    ],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database_error)?;
            let Some(event_bytes) = event_bytes else {
                // No event row for this confirmed_event_seq — ledger is
                // inconsistent; skip rather than fabricate a timestamp.
                continue;
            };
            let event: EventEnvelope =
                serde_json::from_slice(&event_bytes).map_err(serialization_error)?;
            let elapsed = now.signed_duration_since(event.recorded_at);
            let days_since = elapsed.num_days().max(0) as u32;
            if days_since > threshold_days {
                flags.push(TrustFlag::Stale {
                    claim_id: claim.claim_id.to_string(),
                    days_since_modified: days_since,
                });
            }
        }
        // Stable ordering by claim_id.
        flags.sort_by(|a, b| {
            let key = |flag: &TrustFlag| match flag {
                TrustFlag::Stale { claim_id, .. } => claim_id.clone(),
                _ => String::new(),
            };
            key(a).cmp(&key(b))
        });
        Ok(flags)
    }

    /// Explain a retrieval: which active claim_ids match `query` (substring on
    /// subject|predicate, case-insensitive — same rule as the Console `/search`
    /// handler) and are returned in the top_k, and which were excluded because
    /// they were beyond top_k. `reason` is a stable audit string. Deterministic
    /// ordering: matches preserve `all_claims_current`'s iteration order, which
    /// is itself stable on the (claim_status PK, event_seq) read order.
    pub fn retrieval_trace(
        &self,
        query: &str,
        top_k: usize,
        head: u64,
        now: DateTime<Utc>,
    ) -> Result<RetrievalTrace> {
        let current = self.all_claims_current(head, now)?;
        let needle = query.to_lowercase();
        let mut matched: Vec<String> = Vec::new();
        for claim in &current.active {
            if claim.subject.to_lowercase().contains(&needle)
                || claim.predicate.to_lowercase().contains(&needle)
            {
                matched.push(claim.claim_id.to_string());
            }
        }
        let cutoff = top_k.min(matched.len());
        let included_claim_ids: Vec<String> = matched[..cutoff].to_vec();
        let excluded_claim_ids: Vec<String> = matched[cutoff..].to_vec();
        Ok(RetrievalTrace {
            included_claim_ids,
            excluded_claim_ids,
            reason: format!(
                "top_k={top_k} substring match on subject|predicate (matched {})",
                matched.len()
            ),
        })
    }

    /// List every registered client with their last-activity timestamp and
    /// mutation count. `last_active_at` is the MAX(recorded_at) of any event
    /// attributed to that client; `mutation_count` counts the event types that
    /// change ledger state (claim lifecycle + entity rewrite + purge). NEVER
    /// infers person identity (TM-024) — only client_id + label.
    pub fn list_clients(&self) -> Result<Vec<ClientActivity>> {
        let _maintenance = self.coordinator.maintenance.read();
        let connection = open_connection(&self.root)?;
        // Mutation event types: every event_type that changes ledger state
        // (the same set the operations dashboard counts as "mutations").
        const MUTATION_EVENT_TYPES: &[&str] = &[
            "claim_confirmed",
            "claim_rejected",
            "claim_superseded",
            "claim_retracted",
            "entity_merged",
            "entity_renamed",
            "entity_split",
            "registry_denied",
        ];
        let placeholders = std::iter::repeat_n("?", MUTATION_EVENT_TYPES.len())
            .collect::<Vec<_>>()
            .join(",");
        let mut clients_statement = connection
            .prepare("SELECT client_id, label FROM clients ORDER BY label ASC")
            .map_err(database_error)?;
        let client_rows = clients_statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database_error)?;
        let mut clients: Vec<(String, String)> = Vec::new();
        for row in client_rows {
            clients.push(row.map_err(database_error)?);
        }
        drop(clients_statement);

        // Capabilities: best-effort read from client_capabilities. Stored as
        // one row per (client_id, capability); collected into a Vec<String>.
        let mut caps_statement = connection
            .prepare("SELECT capability FROM client_capabilities WHERE client_id=?1 ORDER BY capability ASC")
            .map_err(database_error)?;
        // last_active_at = MAX(recorded_at) of any event for this client.
        // Returns NULL when the client has no events (e.g. the bootstrap
        // client before any mutation), so we read into Option<String> and
        // fall back to "never".
        let mut last_active_statement = connection
            .prepare(
                "SELECT MAX(recorded_at) FROM ( \
                   SELECT json_extract(event_json, '$.recorded_at') AS recorded_at \
                   FROM events WHERE json_extract(event_json, '$.client_id')=?1 \
                 )",
            )
            .map_err(database_error)?;
        // mutation_count = COUNT(*) of mutation events for this client.
        let mutation_sql = format!(
            "SELECT COUNT(*) FROM events \
             WHERE json_extract(event_json, '$.client_id')=?1 \
             AND event_type IN ({placeholders})"
        );
        let mut mutation_statement = connection.prepare(&mutation_sql).map_err(database_error)?;

        let mut out = Vec::with_capacity(clients.len());
        for (client_id, label) in clients {
            let capabilities: Vec<String> = caps_statement
                .query_map([client_id.clone()], |row| row.get::<_, String>(0))
                .map_err(database_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(database_error)?;
            let last_active_at: Option<Option<String>> = last_active_statement
                .query_row([client_id.clone()], |row| row.get::<_, Option<String>>(0))
                .optional()
                .map_err(database_error)?;
            let last_active_at = last_active_at.flatten();
            let mutation_count: i64 = mutation_statement
                .query_row(
                    rusqlite::params_from_iter(
                        std::iter::once(client_id.as_str())
                            .chain(MUTATION_EVENT_TYPES.iter().copied()),
                    ),
                    |row| row.get(0),
                )
                .unwrap_or(0);
            let mutation_count = u64::try_from(mutation_count).unwrap_or(0);
            out.push(ClientActivity {
                client_id,
                label,
                capabilities,
                last_active_at: last_active_at.unwrap_or_else(|| "never".to_owned()),
                mutation_count,
            });
        }
        Ok(out)
    }

    /// Register an in-process async job (Phase E Task E3.1). The job starts in
    /// status "queued"; callers transition it via [`Self::complete_job`] /
    /// [`Self::fail_job`]. Returns the new job id. Per-instance (not persisted):
    /// the dashboard reports current queue depth, not historical runs.
    pub fn register_job(&self, kind: &str) -> Result<String> {
        let id = format!("job-{}", Uuid::now_v7().simple());
        let record = JobRecord {
            id: id.clone(),
            kind: kind.to_owned(),
            status: "queued".to_owned(),
            started_at: self.clock.now(),
        };
        self.jobs.lock().insert(id.clone(), record);
        Ok(id)
    }

    /// Transition a registered job to "active" (optional intermediate state).
    pub fn activate_job(&self, job_id: &str) -> Result<()> {
        let mut jobs = self.jobs.lock();
        if let Some(record) = jobs.get_mut(job_id) {
            record.status = "active".to_owned();
        }
        Ok(())
    }

    /// Mark a registered job as completed (removes it from the active queue).
    pub fn complete_job(&self, job_id: &str) -> Result<()> {
        let mut jobs = self.jobs.lock();
        if let Some(record) = jobs.get_mut(job_id) {
            record.status = "completed".to_owned();
            jobs.remove(job_id);
        }
        Ok(())
    }

    /// Mark a registered job as failed. Failed jobs are retained in the
    /// registry (status="failed") so the dashboard can surface them until an
    /// operator clears the registry by restarting the process.
    pub fn fail_job(&self, job_id: &str) -> Result<()> {
        let mut jobs = self.jobs.lock();
        if let Some(record) = jobs.get_mut(job_id) {
            record.status = "failed".to_owned();
        }
        Ok(())
    }

    /// Summarize the in-process async-job queue. With no Phase E publisher
    /// wiring in yet (E3.1 ships the registry only), a fresh store reports
    /// `{active:0, queued:0, failed:0}` — the contract's "no jobs running"
    /// state. Once publishers register jobs, this surface reflects live depth.
    pub fn job_summary(&self) -> Result<JobSummary> {
        let jobs = self.jobs.lock();
        let mut active = 0u32;
        let mut queued = 0u32;
        let mut failed = 0u32;
        for record in jobs.values() {
            match record.status.as_str() {
                "active" => active += 1,
                "queued" => queued += 1,
                "failed" => failed += 1,
                _ => {}
            }
        }
        Ok(JobSummary {
            active,
            queued,
            failed,
        })
    }

    /// Backup health for the operations dashboard. `last_backup_at` is the
    /// creation time of the most recent non-invalidated row in
    /// `purge_backup_sets` (every `backup_consistent` call inserts one); if no
    /// backup has ever been made, returns `"never"`. `last_restore_drill_ok`
    /// reads `<state_dir>/restore-drill.json` if present (Phase F3 populates
    /// this file); absent the file, returns `false`. For E3.1 no drill has ever
    /// run, so the field is `false` until F3 wires it in.
    pub fn backup_health(&self) -> Result<BackupHealth> {
        let connection = open_connection(&self.root)?;
        let last_backup_at: Option<String> = connection
            .query_row(
                "SELECT MAX(created_at) FROM purge_backup_sets WHERE invalidated_at IS NULL",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error)?
            .flatten();
        let last_backup_at = last_backup_at.unwrap_or_else(|| "never".to_owned());

        // Phase F3 dependency: restore-drill state file. The store's root
        // doubles as the state dir for operator-managed artifacts. Absent the
        // file, no drill has run → false.
        let drill_path = self.root.join("restore-drill.json");
        let last_restore_drill_ok = if drill_path.is_file() {
            let body = fs::read_to_string(&drill_path).map_err(io_error)?;
            serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|value| value.get("last_ok").and_then(|flag| flag.as_bool()))
                .unwrap_or(false)
        } else {
            false
        };
        Ok(BackupHealth {
            last_backup_at,
            last_restore_drill_ok,
        })
    }

    /// Summarize the last domain-eval run for `domain`. Phase E Task E3.1
    /// introduces an in-DB `domain_eval_runs` table so future Phase 4.3
    /// publishers can record runs; for E3.1 no publisher exists yet, so a fresh
    /// store returns `{case_count:0, passed:0, abstention_passed:false,
    /// run_at:"never"}`. The schema is created lazily (idempotent `CREATE TABLE
    /// IF NOT EXISTS`) so older stores upgrade transparently on first read.
    pub fn eval_summary(&self, domain: &str) -> Result<EvalSummary> {
        let connection = open_connection(&self.root)?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS domain_eval_runs(\
                   run_id TEXT PRIMARY KEY,\
                   domain TEXT NOT NULL,\
                   case_count INTEGER NOT NULL,\
                   passed INTEGER NOT NULL,\
                   abstention_passed INTEGER NOT NULL,\
                   run_at TEXT NOT NULL\
                 )",
                [],
            )
            .map_err(database_error)?;
        let row: Option<(i64, i64, i64, String)> = connection
            .query_row(
                "SELECT case_count, passed, abstention_passed, run_at \
                 FROM domain_eval_runs WHERE domain=?1 \
                 ORDER BY run_at DESC LIMIT 1",
                [domain],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(database_error)?;
        match row {
            Some((case_count, passed, abstention_passed, run_at)) => Ok(EvalSummary {
                case_count: u32::try_from(case_count).unwrap_or(0),
                passed: u32::try_from(passed).unwrap_or(0),
                abstention_passed: abstention_passed != 0,
                run_at,
            }),
            None => Ok(EvalSummary {
                case_count: 0,
                passed: 0,
                abstention_passed: false,
                run_at: "never".to_owned(),
            }),
        }
    }

    /// Record a domain-eval run (publisher helper for Phase 4.3 eval drivers).
    /// Provided in E3.1 so future publishers can write through the same store
    /// surface that [`Self::eval_summary`] reads; no internal caller yet.
    pub fn record_eval_run(
        &self,
        domain: &str,
        case_count: u32,
        passed: u32,
        abstention_passed: bool,
        run_at: DateTime<Utc>,
    ) -> Result<()> {
        let _maintenance = self.coordinator.maintenance.read();
        let _writer = self.coordinator.writer.lock();
        let connection = open_connection(&self.root)?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS domain_eval_runs(\
                   run_id TEXT PRIMARY KEY,\
                   domain TEXT NOT NULL,\
                   case_count INTEGER NOT NULL,\
                   passed INTEGER NOT NULL,\
                   abstention_passed INTEGER NOT NULL,\
                   run_at TEXT NOT NULL\
                 )",
                [],
            )
            .map_err(database_error)?;
        let run_id = format!("eval-{}", Uuid::now_v7().simple());
        connection
            .execute(
                "INSERT INTO domain_eval_runs(run_id,domain,case_count,passed,abstention_passed,run_at) \
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    run_id,
                    domain,
                    case_count as i64,
                    passed as i64,
                    if abstention_passed { 1 } else { 0 },
                    run_at.to_rfc3339(),
                ],
            )
            .map_err(database_error)?;
        Ok(())
    }

    /// Split an entity by predicate (Phase E Task E3.1 — the mirror of
    /// [`Self::merge_entities`]). For each `PredicateAssignment` in `command`,
    /// every claim currently attached to `source_entity_id` whose predicate
    /// matches is rewritten onto that assignment's `target_entity_id`.
    /// Predicates NOT listed in any assignment stay on the source. The source
    /// entity is preserved (NOT deleted) — it retains its residual claims and
    /// remains addressable, so the operation is fully reversible via retract on
    /// the emitted `entity_split` event. Idempotent on `operation_id` via the
    /// same `operations`-table reservation merge_entities uses.
    pub fn split_entities(
        &self,
        context: &TrustedContext,
        command: SplitCommand,
    ) -> Result<MutationOutcome> {
        // Reject the degenerate "self-assignment" early — same shape as
        // merge_entities' self-merge guard.
        for assignment in &command.assignments {
            if assignment.target_entity_id == command.source_entity_id {
                return Err(SemanticError::InvalidTransition(format!(
                    "cannot split entity {} onto itself (predicate {})",
                    command.source_entity_id, assignment.predicate
                )));
            }
        }
        // Reject divergent duplicate predicates: two `PredicateAssignment`s with
        // the SAME predicate but DIFFERENT `target_entity_id` would otherwise be
        // silently collapsed by the `.iter().find(...)` rewrite loop (first one
        // wins), which is a data-loss footgun. Same predicate + same target is
        // harmless and dedups silently.
        let mut predicate_targets: HashMap<String, Uuid> = HashMap::new();
        for assignment in &command.assignments {
            match predicate_targets.get(&assignment.predicate) {
                Some(existing) if *existing != assignment.target_entity_id => {
                    return Err(SemanticError::InvalidTransition(format!(
                        "duplicate predicate in split assignments: {}",
                        assignment.predicate
                    )));
                }
                _ => {
                    predicate_targets
                        .entry(assignment.predicate.clone())
                        .or_insert(assignment.target_entity_id);
                }
            }
        }
        let request_hash = request_hash("split_entities", &command)?;
        let source = command.source_entity_id;
        // Snapshot the assignments into the closure (FnOnce-equivalent through
        // the &F the mutate helper expects).
        let assignments = command.assignments.clone();
        let outcome = self.mutate(
            context,
            &command.operation_id,
            &request_hash,
            Some("confirm"),
            move |transaction, _identity| {
                // Load source row to confirm it exists and capture its domain.
                let source_row: Option<(String, String)> = transaction
                    .query_row(
                        "SELECT domain, canonical_subject FROM entities WHERE entity_id=?1",
                        [source.to_string()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(database_error)?;
                let (source_domain, _source_subject) = source_row.ok_or_else(|| {
                    SemanticError::MissingDependency(format!("entity {source}"))
                })?;

                // Validate every target exists AND lives in the same domain as
                // the source — cross-domain split is rejected just like
                // cross-domain merge.
                let mut target_domains: HashMap<Uuid, String> = HashMap::new();
                for assignment in &assignments {
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        target_domains.entry(assignment.target_entity_id)
                    {
                        let target_domain: String = transaction
                            .query_row(
                                "SELECT domain FROM entities WHERE entity_id=?1",
                                [assignment.target_entity_id.to_string()],
                                |row| row.get(0),
                            )
                            .optional()
                            .map_err(database_error)?
                            .ok_or_else(|| {
                                SemanticError::MissingDependency(format!(
                                    "entity {}",
                                    assignment.target_entity_id
                                ))
                            })?;
                        if target_domain != source_domain {
                            return Err(SemanticError::InvalidTransition(format!(
                                "cannot split across domains: source={source_domain}, target={target_domain}"
                            )));
                        }
                        entry.insert(target_domain);
                    }
                }

                // Pull every claim currently on the source so we can both
                // decide which to move AND count the residual set. predicate is
                // stored on claim_status so we do not need to decrypt payloads.
                let mut claim_statement = transaction
                    .prepare(
                        "SELECT claim_id, predicate FROM claim_status WHERE entity_id=?1",
                    )
                    .map_err(database_error)?;
                let claim_rows = claim_statement
                    .query_map([source.to_string()], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(database_error)?;
                let mut source_claims: Vec<(String, String)> = Vec::new();
                for row in claim_rows {
                    source_claims.push(row.map_err(database_error)?);
                }
                drop(claim_statement);

                // Apply each rewrite. We track moved_claims for the event
                // payload + the SplitOutcome-shaped audit (the outcome is
                // reconstructed from the event payload by callers that want
                // the structured view; the MutationOutcome here carries the
                // raw event per the merge_entities precedent).
                let mut moved_claims: Vec<MovedClaim> = Vec::new();
                for (claim_id_text, predicate) in &source_claims {
                    let Some(assignment) = assignments
                        .iter()
                        .find(|assignment| assignment.predicate == *predicate)
                    else {
                        continue;
                    };
                    let moved = transaction
                        .execute(
                            "UPDATE claim_status SET entity_id=?2 WHERE claim_id=?1",
                            params![claim_id_text, assignment.target_entity_id.to_string()],
                        )
                        .map_err(database_error)?;
                    if moved > 0 {
                        let claim_id = Uuid::parse_str(claim_id_text).map_err(|_| {
                            SemanticError::CorruptLedger(
                                "claim_status.claim_id is not a UUID".to_owned(),
                            )
                        })?;
                        moved_claims.push(MovedClaim {
                            claim_id,
                            predicate: predicate.clone(),
                            from_entity_id: source,
                            to_entity_id: assignment.target_entity_id,
                        });
                    }
                }

                let source_remaining_claim_count = source_claims
                    .iter()
                    .filter(|(_, predicate)| {
                        !assignments.iter().any(|assignment| assignment.predicate == *predicate)
                    })
                    .count() as u64;

                let payload = serde_json::json!({
                    "kind": "entity_split",
                    "domain": source_domain,
                    "source_entity_id": source,
                    "assignments": assignments.iter().map(|a| serde_json::json!({
                        "predicate": a.predicate,
                        "target_entity_id": a.target_entity_id,
                    })).collect::<Vec<_>>(),
                    "moved_claims": moved_claims.iter().map(|m| serde_json::json!({
                        "claim_id": m.claim_id,
                        "predicate": m.predicate,
                        "from_entity_id": m.from_entity_id,
                        "to_entity_id": m.to_entity_id,
                    })).collect::<Vec<_>>(),
                    "source_remaining_claim_count": source_remaining_claim_count,
                });
                Ok(MutationMaterial {
                    event_type: "entity_split",
                    object_bytes: canonical_bytes(&payload)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds::default(),
                })
            },
        )?;
        Ok(outcome)
    }

    /// Structured view over the most recent `entity_split` event for a source
    /// entity (helper for the Console preview surface). Re-derives the
    /// `SplitOutcome` shape from the persisted event payload. Returns `None`
    /// if no split event references this source.
    pub fn last_split_outcome_for(
        &self,
        context: &TrustedContext,
        source_entity_id: Uuid,
    ) -> Result<Option<SplitOutcome>> {
        validate_context(&self.marker, context)?;
        let _maintenance = self.coordinator.maintenance.read();
        let connection = open_connection(&self.root)?;
        // Scan entity_split events in descending seq order; decrypt + parse
        // each payload until one references the requested source. Splits are
        // rare administrative events, so the bounded scan stays cheap.
        let mut statement = connection
            .prepare(
                "SELECT event_seq, object_id FROM events \
                 WHERE owner_id=?1 AND event_type='entity_split' \
                 ORDER BY event_seq DESC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![self.marker.owner_id.to_string()], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database_error)?;
        for row in rows {
            let (event_seq, object_id) = row.map_err(database_error)?;
            let object_bytes = decrypt_object(&connection, &self.root, &object_id)?;
            let value: Value =
                serde_json::from_slice(&object_bytes).map_err(serialization_error)?;
            let payload_source = value
                .get("source_entity_id")
                .and_then(|value| value.as_str())
                .and_then(|text| Uuid::parse_str(text).ok());
            if payload_source != Some(source_entity_id) {
                continue;
            }
            let event = self.event_at(u64::try_from(event_seq).map_err(|_| {
                SemanticError::CorruptLedger("negative event sequence".to_owned())
            })?)?;
            let Some(event) = event else { continue };
            let source_remaining_claim_count = value
                .get("source_remaining_claim_count")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let moved_claims = value
                .get("moved_claims")
                .and_then(|value| value.as_array())
                .map(|array| {
                    array
                        .iter()
                        .filter_map(|item| {
                            let claim_id = item
                                .get("claim_id")
                                .and_then(|value| value.as_str())
                                .and_then(|text| Uuid::parse_str(text).ok())?;
                            let predicate = item
                                .get("predicate")
                                .and_then(|value| value.as_str())?
                                .to_owned();
                            let from_entity_id = item
                                .get("from_entity_id")
                                .and_then(|value| value.as_str())
                                .and_then(|text| Uuid::parse_str(text).ok())?;
                            let to_entity_id = item
                                .get("to_entity_id")
                                .and_then(|value| value.as_str())
                                .and_then(|text| Uuid::parse_str(text).ok())?;
                            Some(MovedClaim {
                                claim_id,
                                predicate,
                                from_entity_id,
                                to_entity_id,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            return Ok(Some(SplitOutcome {
                source_remaining_claim_count,
                moved_claims,
                event,
            }));
        }
        Ok(None)
    }

    /// Look up the persisted `EventEnvelope` at a specific event_seq. Used by
    /// the structured split-outcome view to attach the raw audit event.
    fn event_at(&self, event_seq: u64) -> Result<Option<EventEnvelope>> {
        let connection = open_connection(&self.root)?;
        let bytes: Option<Vec<u8>> = connection
            .query_row(
                "SELECT event_json FROM events WHERE owner_id=?1 AND event_seq=?2",
                params![self.marker.owner_id.to_string(), event_seq as i64],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error)?;
        match bytes {
            Some(bytes) => {
                let event: EventEnvelope =
                    serde_json::from_slice(&bytes).map_err(serialization_error)?;
                Ok(Some(event))
            }
            None => Ok(None),
        }
    }
}

/// Narrow application-core interface for future transport adapters.
pub trait SemanticApplicationCore {
    fn capture(&self, context: &TrustedContext, command: CaptureCommand)
    -> Result<MutationOutcome>;
    fn propose(&self, context: &TrustedContext, command: ProposeCommand)
    -> Result<MutationOutcome>;
    fn confirm(&self, context: &TrustedContext, command: ConfirmCommand)
    -> Result<MutationOutcome>;
}

impl SemanticApplicationCore for SemanticStore {
    fn capture(
        &self,
        context: &TrustedContext,
        command: CaptureCommand,
    ) -> Result<MutationOutcome> {
        SemanticStore::capture(self, context, command)
    }

    fn propose(
        &self,
        context: &TrustedContext,
        command: ProposeCommand,
    ) -> Result<MutationOutcome> {
        SemanticStore::propose(self, context, command)
    }

    fn confirm(
        &self,
        context: &TrustedContext,
        command: ConfirmCommand,
    ) -> Result<MutationOutcome> {
        SemanticStore::confirm(self, context, command)
    }
}

impl StoreAdmin {
    pub fn rollback(&self) -> Result<RollbackStatus> {
        if !self.root.exists() {
            return Ok(RollbackStatus::AlreadyRemoved);
        }
        let _exclusive = self
            .coordinator
            .maintenance
            .try_write()
            .ok_or(SemanticError::ActiveHandles)?;
        if self.coordinator.active_handles.load(Ordering::SeqCst) != 0
            || self.coordinator.active_transactions.load(Ordering::SeqCst) != 0
        {
            return Err(SemanticError::ActiveHandles);
        }
        validate_existing_root(&self.root, &self.allowed_parent)?;
        reject_tree_links(&self.root)?;
        let marker = read_marker(&self.root)?;
        if marker.store_uuid != self.store_uuid || marker.deletion_nonce != self.deletion_nonce {
            return Err(SemanticError::MarkerMismatch);
        }
        let mut connection = open_connection(&self.root)?;
        validate_database_identity(&connection, &marker, GateBehavior::Serve)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        transaction.commit().map_err(database_error)?;
        drop(connection);
        fs::remove_dir_all(&self.root).map_err(io_error)?;
        Ok(RollbackStatus::Removed)
    }
}

fn validate_requested_root(root: &Path, config: &SemanticConfig) -> Result<(PathBuf, PathBuf)> {
    if !config.enabled {
        return Err(SemanticError::Disabled);
    }
    let parent = config
        .allowed_parent
        .as_ref()
        .ok_or_else(|| SemanticError::InvalidRoot("allowed parent is required".to_owned()))?;
    reject_link_or_reparse(parent)?;
    let allowed_parent = parent.canonicalize().map_err(io_error)?;
    reject_hazardous_path(&allowed_parent)?;
    let requested = if root.is_absolute() {
        root.to_path_buf()
    } else {
        allowed_parent.join(root)
    };
    if requested
        .parent()
        .and_then(|value| value.canonicalize().ok())
        .as_deref()
        != Some(allowed_parent.as_path())
    {
        return Err(SemanticError::InvalidRoot(
            "root must be a direct child of allowed parent".to_owned(),
        ));
    }
    reject_hazardous_path(&requested)?;
    if requested.exists() {
        reject_link_or_reparse(&requested)?;
    }
    Ok((requested, allowed_parent))
}

fn validate_existing_root(root: &Path, allowed_parent: &Path) -> Result<()> {
    reject_link_or_reparse(root)?;
    let canonical = root.canonicalize().map_err(io_error)?;
    if canonical != root || canonical.parent() != Some(allowed_parent) {
        return Err(SemanticError::InvalidRoot(
            "rollback root escaped capability boundary".to_owned(),
        ));
    }
    reject_hazardous_path(&canonical)
}

fn reject_hazardous_path(path: &Path) -> Result<()> {
    if path.parent().is_none() {
        return Err(SemanticError::InvalidRoot(
            "filesystem roots are forbidden".to_owned(),
        ));
    }
    for component in path.components() {
        if let Component::Normal(value) = component
            && value.to_string_lossy().eq_ignore_ascii_case(".git")
        {
            return Err(SemanticError::InvalidRoot(
                "repository metadata paths are forbidden".to_owned(),
            ));
        }
    }
    if path.join(".git").is_dir() {
        return Err(SemanticError::InvalidRoot(
            "repository roots are forbidden".to_owned(),
        ));
    }
    Ok(())
}

fn reject_link_or_reparse(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            continue;
        };
        if metadata_is_link_or_reparse(&metadata) {
            return Err(SemanticError::InvalidRoot(
                "symbolic links and reparse points are forbidden".to_owned(),
            ));
        }
    }
    Ok(())
}

fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

fn reject_tree_links(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root).map_err(io_error)? {
        let path = entry.map_err(io_error)?.path();
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(SemanticError::InvalidRoot(
                "store interior contains a symbolic link or reparse point".to_owned(),
            ));
        }
        if metadata.is_dir() {
            reject_tree_links(&path)?;
        }
    }
    Ok(())
}

fn read_marker(root: &Path) -> Result<StoreMarker> {
    let bytes = fs::read(root.join(MARKER_FILE)).map_err(|_| SemanticError::MarkerMismatch)?;
    serde_json::from_slice(&bytes).map_err(|_| SemanticError::MarkerMismatch)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_error)?;
    file.write_all(bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

/// Creates a new owner-only secret file (Task F3.1, hardened in F3.2 carry
/// fix 1). The Unix arm opens with `OpenOptionsExt::mode(0o600)` so the file
/// is created with the right mode ATOMICALLY — there is no window where the
/// file exists on disk in a default (potentially world-readable) mode. A
/// chmod failure (rare but possible on a broken ACL inheritance) is now
/// fail-closed: the create returns `Err` and the key file is never left
/// world-readable. On Windows there is no equivalent std API, so the file
/// inherits the user-profile default ACL (user-only in a normal profile; see
/// the [`SemanticStore::load_or_create_backup_key`] docs for the operator
/// `icacls` recipe).
fn write_new_secret_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
        }
        #[cfg(not(unix))]
        {
            OpenOptions::new().write(true).create_new(true).open(path)
        }
    }
    .map_err(|error| {
        SemanticError::Io(format!(
            "failed to create secret file {}: {error}",
            path.display()
        ))
    })?;
    file.write_all(bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

/// Encrypts one plaintext file into `target` as `nonce || ciphertext` under
/// the supplied AES-256-GCM cipher (Task F3.1). Used by
/// [`SemanticStore::backup_encrypted`] for every backup layer (db, marker,
/// projection, object blobs). The nonce is a fresh random 12-byte value per
/// file via `OsRng` — negligible collision risk at backup cadence.
fn encrypt_file_into(cipher: &Aes256Gcm, source: &Path, target: &Path) -> Result<()> {
    let plaintext = fs::read(source).map_err(io_error)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_slice())
        .map_err(|_| SemanticError::Serialization("backup layer encryption failed".to_owned()))?;
    let mut envelope = Vec::with_capacity(AES_GCM_NONCE_LEN + ciphertext.len());
    envelope.extend_from_slice(nonce.as_slice());
    envelope.extend_from_slice(&ciphertext);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    write_new_file(target, &envelope)
}

/// Decrypts one `nonce || ciphertext` file produced by [`encrypt_file_into`]
/// back into plaintext bytes. Exposed as a free function so integration tests
/// can perform an independent round-trip (read `backup.key`, decrypt each
/// `.enc` file, compare against a parallel plaintext backup).
pub fn decrypt_backup_layer(cipher: &Aes256Gcm, encrypted_source: &Path) -> Result<Vec<u8>> {
    let envelope = fs::read(encrypted_source).map_err(io_error)?;
    if envelope.len() < AES_GCM_NONCE_LEN {
        return Err(SemanticError::CorruptLedger(format!(
            "encrypted backup layer {} is too short ({} bytes; need at least the \
             12-byte nonce)",
            encrypted_source.display(),
            envelope.len()
        )));
    }
    let (nonce_bytes, ciphertext) = envelope.split_at(AES_GCM_NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| SemanticError::ObjectUnavailable("backup layer decryption failed".to_owned()))
}

/// Opens a snapshot sqlite db (read-only) and counts the ledger events in
/// it (Task F3.1). Run against the staged snapshot rather than the live
/// store so the recorded count matches exactly what a restore would see.
fn count_ledger_events(snapshot_db: &Path) -> Result<u64> {
    let connection = Connection::open(snapshot_db).map_err(database_error)?;
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .map_err(database_error)?;
    Ok(count.max(0) as u64)
}

/// Reads the snapshot db's PurgeRegistry head epoch (Task F3.2). Returns 0
/// when no denial has ever been recorded (matches `composite_checksum`'s own
/// `local_registry_head().map_or(0, ...)` fallback). Run against the staged
/// snapshot so the manifest's recorded epoch matches exactly what a restore
/// would see in the decrypted db layer.
fn snapshot_registry_epoch(connection: &Connection) -> Result<u64> {
    // The table may be absent in degenerate snapshots; COALESCE to 0 rather
    // than erroring so a fresh-store backup (no purge activity) still records
    // a clean manifest.
    let epoch: Option<i64> = connection
        .query_row(
            "SELECT MAX(purge_epoch) FROM purge_registry_entries",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?
        .flatten();
    Ok(epoch.map(|value| value.max(0) as u64).unwrap_or(0))
}

/// Removes the wrapped directory when dropped, regardless of how control
/// leaves the owning scope (panic unwind, early return via `?`, or normal
/// fallthrough). Used to guarantee staging dirs containing decrypted
/// plaintext are wiped even on panic — defense-in-depth for the threat
/// model: a rusqlite internal panic or a stray `.unwrap()` mid-restore must
/// not leave plaintext on disk at `parent/.{target}-restore-staging/` (or
/// the matching `.{target}-staging/` backup sibling).
///
/// Disarm via [`StagingDirGuard::disarm`] once the staging dir has been
/// renamed to its final target (atomic) — the guard's `Drop` also tolerates
/// the path no longer existing (it `.exists()` checks before removal), so
/// disarming after a successful rename is a defensive no-op even if a
/// caller forgets.
struct StagingDirGuard(Option<PathBuf>);

impl StagingDirGuard {
    fn new(path: PathBuf) -> Self {
        Self(Some(path))
    }

    /// Disarm the guard (staging will NOT be removed on drop). Use when the
    /// staging dir has been renamed to its final target.
    fn disarm(mut self) {
        self.0.take();
    }
}

impl Drop for StagingDirGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take()
            && path.exists()
        {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Plaintext manifest written into every encrypted-backup directory (Task
/// F3.1). Carries the layer-list + composite checksum + created-at so a
/// clean-host restore drill can verify the snapshot without first
/// decrypting it. The cipher key is NOT recorded here (chicken-and-egg).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupManifest {
    /// Manifest schema version. Bumped on incompatible shape changes.
    version: u32,
    /// Always `true` for backups produced by `backup_encrypted`.
    encrypted: bool,
    /// Cipher algorithm identifier (e.g. "AES-256-GCM").
    cipher: String,
    /// Composite checksum of the LIVE store at backup time; a restore drill
    /// recomputes this against the restored snapshot and compares.
    composite_checksum: String,
    /// RFC3339 timestamp the backup was taken.
    created_at: String,
    /// Number of object blobs encrypted (informational; restore drills count
    /// actual decrypted blobs and compare).
    objects_count: u64,
    /// Number of ledger events in the snapshot (informational; same caveat).
    ledger_events_count: u64,
    /// PurgeRegistry epoch recorded at backup time (Task F3.2). On restore,
    /// the recomputed `local_registry_head()` of the decrypted db layer MUST
    /// match this epoch — a mismatch means the registry diverged between
    /// backup and restore (e.g. someone purged between snapshot and drill),
    /// which fails the restore-drill closed. Defaults to 0 when missing so
    /// older F3.1 manifests (which had no purge_epoch) parse and report
    /// "no denials recorded", matching the original pre-F3.2 semantics.
    #[serde(default)]
    purge_epoch: u64,
    /// Per-blob plaintext digests (F3.2 review Fix 2). Keyed by object path
    /// relative to the `objects/` root (e.g. `"ab/cd/abcd1234..."`), valued by
    /// `sha256(plaintext_bytes)` (lowercase hex). On restore, each decrypted
    /// blob's recomputed digest MUST equal the manifest's entry — this binds
    /// the AES-GCM-authenticated ciphertext to THIS specific backup, defeating
    /// an attacker who substitutes `objects/<shard>/<digest>.enc` with a
    /// legitimately-encrypted blob from a DIFFERENT object under the same key.
    ///
    /// Defaults to an empty map so F3.1 manifests (no digests) parse; on
    /// restore, an empty map means "no per-blob verification" and a warning
    /// is logged (back-compat — operators re-taking a backup under F3.2 fill
    /// the map and get the stronger guarantee).
    #[serde(default)]
    objects: HashMap<String, String>,
}

fn open_connection(root: &Path) -> Result<Connection> {
    let connection = Connection::open(root.join(DATABASE_FILE)).map_err(database_error)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(database_error)?;
    connection
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(database_error)?;
    Ok(connection)
}

fn initialize_schema(
    connection: &Connection,
    marker: &StoreMarker,
    created_at: DateTime<Utc>,
) -> Result<()> {
    connection
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .map_err(database_error)?;
    connection
        .execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE events(
               owner_id TEXT NOT NULL,
               event_seq INTEGER NOT NULL,
               event_id TEXT NOT NULL UNIQUE,
               event_type TEXT NOT NULL,
               event_json BLOB NOT NULL,
               event_hash TEXT NOT NULL,
               object_id TEXT NOT NULL,
               PRIMARY KEY(owner_id,event_seq)
             );
             CREATE TABLE operations(
               owner_id TEXT NOT NULL,
               client_id TEXT NOT NULL,
               operation_id TEXT NOT NULL,
               request_hash TEXT NOT NULL,
               outcome BLOB,
               PRIMARY KEY(owner_id,client_id,operation_id)
             );
             CREATE TABLE outbox(
               owner_id TEXT NOT NULL,
               event_seq INTEGER NOT NULL,
               acknowledged INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY(owner_id,event_seq),
               FOREIGN KEY(owner_id,event_seq) REFERENCES events(owner_id,event_seq)
             );
             CREATE TABLE clients(
               client_id TEXT PRIMARY KEY,
               label TEXT NOT NULL UNIQUE,
               created_at TEXT NOT NULL
             );
             CREATE TABLE client_capabilities(
               client_id TEXT NOT NULL,
               capability TEXT NOT NULL,
               granted_at TEXT NOT NULL,
               PRIMARY KEY(client_id,capability)
             );
             CREATE TABLE proposal_status(
               proposal_id TEXT PRIMARY KEY,
               status TEXT NOT NULL,
               claim_id TEXT
             );
             -- Status is derived from the three sequence columns against a
             -- requested ledger_head, not stored as a label: a claim is
             -- superseded/retracted as of a given head only when that head
             -- is at or after the recorded transition sequence, which keeps
             -- as-of queries correct even though this table's rows are
             -- updated in place.
             CREATE TABLE claim_status(
               claim_id TEXT PRIMARY KEY,
               domain TEXT NOT NULL,
               subject TEXT NOT NULL,
               predicate TEXT NOT NULL,
               confirmed_event_seq INTEGER NOT NULL,
               superseded_by_event_seq INTEGER,
               retracted_at_event_seq INTEGER,
               -- Task 2.2: stable entity this claim resolves to. Populated at
               -- confirm time; nullable only for rows backfilled from legacy
               -- stores (none in production — Task 2.3 territory).
               entity_id TEXT
             );
             -- Task 2.2 entity model (ADR Decision 3). One stable UUIDv7 per
             -- (domain, canonical_subject). Rename updates canonical_subject
             -- but keeps entity_id; merge rewrites claim_status.entity_id and
             -- turns the source subject into an alias row.
             CREATE TABLE entities(
               entity_id TEXT PRIMARY KEY,
               domain TEXT NOT NULL,
               canonical_subject TEXT NOT NULL,
               created_at TEXT NOT NULL,
               UNIQUE(domain, canonical_subject)
             );
             -- Every subject string (or external id) that has ever resolved to
             -- an entity. kind='canonical' mirrors the current
             -- canonical_subject; kind='former_subject' is a rename/merge
             -- backlink; kind='external' is a provider id alias (Decision 3).
             CREATE TABLE entity_aliases(
               domain TEXT NOT NULL,
               alias TEXT NOT NULL,
               entity_id TEXT NOT NULL,
               kind TEXT NOT NULL,
               aliased_at_event_seq INTEGER NOT NULL,
               PRIMARY KEY(domain, alias, entity_id)
             );
             -- epoch_keys holds the owner's key-encryption-key (KEK) history.
             -- wrapped_keys holds each object's random data-encryption-key
             -- (DEK), itself encrypted under one epoch's KEK. Cryptographic
             -- erasure of a single object is `DELETE FROM wrapped_keys`;
             -- erasure of every object under a whole epoch is destroying
             -- that epoch's row (done automatically by rotate-and-rewrap
             -- once nothing references it any more).
             CREATE TABLE epoch_keys(
               epoch INTEGER PRIMARY KEY,
               key_material BLOB NOT NULL,
               created_at TEXT NOT NULL
             );
             CREATE TABLE wrapped_keys(
               object_id TEXT PRIMARY KEY,
               epoch INTEGER NOT NULL,
               wrapped_dek BLOB NOT NULL,
               dek_nonce BLOB NOT NULL
             );
             -- Append-only, hash-chained purge-registry journal. Each row
             -- is one denial batch; purge_denied_ids is the fast-lookup
             -- projection of every ID denied at or before its purge_epoch,
             -- rebuildable from purge_registry_entries alone.
             CREATE TABLE purge_registry_entries(
               purge_epoch INTEGER PRIMARY KEY,
               denied_ids_json TEXT NOT NULL,
               entry_hash TEXT NOT NULL,
               prior_entry_hash TEXT,
               recorded_at TEXT NOT NULL
             );
             CREATE TABLE purge_denied_ids(
               denied_id TEXT PRIMARY KEY,
               purge_epoch INTEGER NOT NULL
             );
             -- Hard-purge saga state machine (ADR Decision 7): one row per
             -- purge attempt, keyed for idempotent replay like the
             -- ledger's own `operations` table. `state` advances through
             -- requested -> registry_denied -> key_revoked -> live_deleted
             -- -> projections_cleaned -> retention_pending -> completed.
             CREATE TABLE purge_sagas(
               purge_id TEXT PRIMARY KEY,
               client_id TEXT NOT NULL,
               operation_id TEXT NOT NULL,
               request_hash TEXT NOT NULL,
               targets_json TEXT NOT NULL,
               preview_hash TEXT NOT NULL,
               state TEXT NOT NULL,
               registry_epoch INTEGER,
               new_backup_path TEXT,
               composite_checksum TEXT,
               created_at TEXT NOT NULL,
               completed_at TEXT,
               UNIQUE(client_id,operation_id)
             );
             -- Single-use, 60-second preview-hash-bound confirmation nonces.
             CREATE TABLE purge_nonces(
               nonce TEXT PRIMARY KEY,
               preview_hash TEXT NOT NULL,
               targets_json TEXT NOT NULL,
               expires_at TEXT NOT NULL,
               used_at TEXT
             );
             -- Backups this store created via backup_consistent, so a
             -- purge's retention_pending step knows which ones might still
             -- carry a wrapped key for a just-purged object.
             CREATE TABLE purge_backup_sets(
               backup_path TEXT PRIMARY KEY,
               created_at TEXT NOT NULL,
               invalidated_at TEXT
             );
             COMMIT;",
        )
        .map_err(database_error)?;
    for (key, value) in [
        ("store_uuid", marker.store_uuid.to_string()),
        ("owner_id", marker.owner_id.to_string()),
        ("schema_version", marker.schema_version.to_string()),
        ("registry_sealed", "false".to_owned()),
    ] {
        connection
            .execute(
                "INSERT INTO meta(key,value) VALUES (?1,?2)",
                params![key, value],
            )
            .map_err(database_error)?;
    }
    connection
        .execute(
            "INSERT INTO clients(client_id,label,created_at) VALUES (?1,?2,?3)",
            params![
                marker.client_id.to_string(),
                BOOTSTRAP_CLIENT_LABEL,
                created_at.to_rfc3339()
            ],
        )
        .map_err(database_error)?;
    for capability in DEFAULT_CLIENT_CAPABILITIES {
        connection
            .execute(
                "INSERT INTO client_capabilities(client_id,capability,granted_at) VALUES (?1,?2,?3)",
                params![marker.client_id.to_string(), capability, created_at.to_rfc3339()],
            )
            .map_err(database_error)?;
    }
    let first_key = Aes256Gcm::generate_key(&mut OsRng);
    connection
        .execute(
            "INSERT INTO epoch_keys(epoch,key_material,created_at) VALUES (1,?1,?2)",
            params![first_key.as_slice(), created_at.to_rfc3339()],
        )
        .map_err(database_error)?;
    Ok(())
}

fn validate_database_identity(
    connection: &Connection,
    marker: &StoreMarker,
    gate: GateBehavior,
) -> Result<()> {
    let value: String = connection
        .query_row("SELECT value FROM meta WHERE key='store_uuid'", [], |row| {
            row.get(0)
        })
        .map_err(database_error)?;
    if value != marker.store_uuid.to_string() {
        return Err(SemanticError::MarkerMismatch);
    }
    // Schema-version gate (Task 2.2 + Task F3.3). Two cases:
    //
    //   1. marker.schema_version == CURRENT_DISK_SCHEMA_VERSION → serve.
    //
    //   2. The on-disk version is OLDER and a known migration path exists
    //      (today only 2 → 3, the F3.3 noop placeholder). The store refuses
    //      to serve until an operator explicitly runs `llm-wiki recovery
    //      upgrade`. This is refuse-to-serve-until-upgraded (rather than
    //      auto-upgrade on open) so the upgrade is always an explicit,
    //      logged operator action — never a side-effect of starting the
    //      server against an old store. The error message names the binary
    //      version + the CLI subcommand to run.
    //
    //   3. The on-disk version is genuinely unsupported (older than 2, or
    //      newer than the binary). Hard-fail closed, same as Task 2.2's
    //      original gate — a store the binary was never written against
    //      must never silently run under DDL it cannot reason about.
    //
    // The `gate` parameter relaxes case (2) for the upgrade CLI path:
    // `open_for_upgrade` passes `GateBehavior::UpgradeOnly`, which lets a
    // known-path older version through so `plan_schema_upgrade` + friends
    // can run. Case (3) ALWAYS fails closed regardless of gate — a
    // genuinely unsupported version cannot even be upgraded.
    //
    // This compares the on-disk DDL version, not the event wire version.
    if marker.schema_version != CURRENT_DISK_SCHEMA_VERSION {
        let has_path =
            schema_upgrade_path_exists(marker.schema_version, CURRENT_DISK_SCHEMA_VERSION);
        if matches!(gate, GateBehavior::UpgradeOnly) && has_path {
            // The upgrade CLI is explicitly running against an older store
            // whose migration path is known — allow the open so the upgrade
            // can run. The instance's marker carries the OLD version, which
            // is exactly what plan_schema_upgrade(from) expects.
            return Ok(());
        }
        if has_path {
            return Err(SemanticError::CorruptLedger(format!(
                "store schema_version {} is older than binary schema_version {}; a migration \
                 path exists — run `llm-wiki recovery upgrade` (refusing to serve until the \
                 operator explicitly upgrades)",
                marker.schema_version, CURRENT_DISK_SCHEMA_VERSION
            )));
        }
        return Err(SemanticError::CorruptLedger(format!(
            "store schema_version {} does not match binary schema_version {} (no migration \
             path yet — pre-production break)",
            marker.schema_version, CURRENT_DISK_SCHEMA_VERSION
        )));
    }
    Ok(())
}

/// Which schema-version-gate branch the opener wants enforced. See
/// [`SemanticStore::open_with_gate_behavior`].
#[derive(Clone, Copy, Debug)]
enum GateBehavior {
    /// Running-server path: refuse to serve an older-version store even
    /// when a migration path exists (operator must run `recovery upgrade`
    /// first).
    Serve,
    /// Upgrade-CLI path: let a known-migration-path older version through
    /// so the upgrade can run. Genuinely unsupported versions still fail.
    UpgradeOnly,
}

/// True iff `plan_schema_upgrade(from, to)` would produce a plan (i.e. a
/// known migration path exists for this version pair). Today: `2 → 3`
/// (the F3.3 noop placeholder) and `3 → 4` (the Entity Identity Reform
/// path — wired as a noop plan for now; Task 2 swaps the step body for the
/// genuine migration). Used by `validate_database_identity` to
/// distinguish "refuse-to-serve-until-upgraded" (known path) from
/// "genuinely unsupported version" (no path) — both still fail closed, but
/// the error message differs so an operator sees the actionable next step.
fn schema_upgrade_path_exists(from: u8, to: u8) -> bool {
    matches!((from, to), (2, 3) | (3, 4))
}

/// Runs one `UpgradeStep`'s forward action inside an open transaction
/// (Task F3.3). The v2→v3 step is a noop placeholder — there is no DDL to
/// apply, no rows to backfill, no JSON shape to migrate — so the body just
/// records that the step ran by inserting a one-row audit log into
/// `meta` under a stable key. The audit row is itself inside the same
/// transaction, so a step failure rolls it back alongside the (future)
/// DDL change. The `forward_index` lets a multi-step plan record per-step
/// progress (`upgrade_step_<index>_at`).
///
/// Future genuine migrations: add an arm keyed on `(from_version,
/// to_version, step.description)` here. Each arm runs its DDL/data migration
/// against `transaction` and returns `Err` on any failure — the caller's
/// `?` propagates and rusqlite's `Transaction` Drop issues ROLLBACK.
fn run_upgrade_step_forward(
    transaction: &Transaction,
    from_version: u8,
    to_version: u8,
    forward_index: usize,
    step: &crate::recovery::UpgradeStep,
    now: DateTime<Utc>,
) -> Result<()> {
    // Phase Reform Task 3/4: the genuine v3→v4 migration. Runs entirely
    // inside this transaction — any failure rolls back via `?`.
    if (from_version, to_version) == (3, 4) && forward_index == 0 {
        run_entity_identity_reform_forward(transaction)?;
    }

    // Audit-trail row: records which step ran, against which version pair,
    // and whether the step advertises itself reversible. The value is the
    // step description (operator-readable in `SELECT key,value FROM meta`).
    let audit_key = format!("upgrade_step_{forward_index}_to_v{to_version}");
    transaction
        .execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES (?1, ?2)",
            params![
                audit_key,
                format!(
                    "{{\"from\":{from_version},\"to\":{to_version},\"reversible\":{},\"description\":\"{}\",\"at\":\"{}\"}}",
                    step.reversible,
                    step.description.replace('"', "\\\""),
                    now.to_rfc3339()
                )
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

/// Entity Identity Reform forward migration (v3 → v4). Executes the four
/// consolidation steps from the design doc §7.1, all inside the caller's
/// transaction:
///
///   1. Pick a canonical target per `canonical_subject` (most active claims wins;
///      ties broken by lexicographically smallest entity_id for determinism).
///   2. Rewrite every `claim_status.entity_id` from a losing entity onto its
///      target.
///   3. Fold losing entities' aliases onto their target as `former_subject`.
///   4. Delete the losing entities.
///
/// The constraint change (dropping `domain` from the entities UNIQUE and the
/// entity_aliases PK) is performed in Task 4, immediately after this returns,
/// inside the same transaction.
fn run_entity_identity_reform_forward(transaction: &Transaction) -> Result<()> {
    // Step 1: pick the canonical target per canonical_subject.
    let mut stmt = transaction
        .prepare(
            "SELECT entity_id, canonical_subject FROM entities ORDER BY canonical_subject ASC, entity_id ASC",
        )
        .map_err(database_error)?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(database_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(database_error)?;
    drop(stmt);

    // Active-claim count per entity (superseded/retracted claims do not count
    // toward "most claims wins" — only the live ones do).
    let mut claim_counts: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();
    {
        let mut stmt = transaction
            .prepare(
                "SELECT entity_id, COUNT(*) FROM claim_status \
                 WHERE superseded_by_event_seq IS NULL AND retracted_at_event_seq IS NULL \
                 GROUP BY entity_id",
            )
            .map_err(database_error)?;
        let counts = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(database_error)?;
        for c in counts {
            let (eid, n) = c.map_err(database_error)?;
            claim_counts.insert(eid, n);
        }
    }

    // Group by canonical_subject; pick target = most claims, ties → smallest entity_id.
    let mut groups: std::collections::HashMap<String, Vec<(String, i64)>> =
        std::collections::HashMap::new();
    for (eid, subject) in &rows {
        groups
            .entry(subject.clone())
            .or_default()
            .push((eid.clone(), *claim_counts.get(eid).unwrap_or(&0)));
    }
    let mut targets: Vec<(String, String)> = Vec::new(); // (canonical_subject, target_entity_id)
    let mut losers: Vec<String> = Vec::new(); // entity_ids to delete
    for (subject, mut members) in groups {
        // Sort: most claims first. The rows were pre-sorted by entity_id ASC,
        // so a stable sort preserves entity_id as the tiebreak.
        members.sort_by(|a, b| b.1.cmp(&a.1));
        let target = members[0].0.clone();
        targets.push((subject, target));
        for (eid, _) in &members[1..] {
            losers.push(eid.clone());
        }
    }

    if losers.is_empty() {
        return Ok(()); // nothing to consolidate; Task 4's constraint step still runs.
    }

    // Step 2: rewrite claim_status.entity_id from each loser onto its target.
    let subject_of: std::collections::HashMap<String, String> = rows
        .iter()
        .cloned()
        .collect(); // entity_id → canonical_subject
    let target_of_subject: std::collections::HashMap<&str, &str> = targets
        .iter()
        .map(|(s, t)| (s.as_str(), t.as_str()))
        .collect();
    for loser in &losers {
        let subject = subject_of.get(loser).expect("loser subject");
        let target = target_of_subject
            .get(subject.as_str())
            .copied()
            .expect("target for subject");
        transaction
            .execute(
                "UPDATE claim_status SET entity_id=?1 WHERE entity_id=?2",
                params![target, loser],
            )
            .map_err(database_error)?;
    }

    // Step 3: fold aliases from losers onto their targets as former_subject.
    for loser in &losers {
        let subject = subject_of.get(loser).expect("loser subject");
        let target = target_of_subject
            .get(subject.as_str())
            .copied()
            .expect("target");
        let mut stmt = transaction
            .prepare("SELECT alias FROM entity_aliases WHERE entity_id=?1")
            .map_err(database_error)?;
        let aliases: Vec<String> = stmt
            .query_map([loser], |row| row.get::<_, String>(0))
            .map_err(database_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(database_error)?;
        drop(stmt);
        for alias in aliases {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO entity_aliases(domain, alias, entity_id, kind, aliased_at_event_seq) \
                     VALUES (?1, ?2, ?3, 'former_subject', 0)",
                    params![subject, alias, target],
                )
                .map_err(database_error)?;
        }
    }

    // Step 4: delete the losing entities + their alias rows.
    for loser in &losers {
        transaction
            .execute(
                "DELETE FROM entity_aliases WHERE entity_id=?1",
                [loser],
            )
            .map_err(database_error)?;
        transaction
            .execute("DELETE FROM entities WHERE entity_id=?1", [loser])
            .map_err(database_error)?;
    }

    Ok(())
}

/// Runs one `UpgradeStep`'s reverse action inside an open transaction. The
/// v2→v3 noop reverse is also a noop for the data layer, but it DOES wipe
/// the per-step audit row written by `run_upgrade_step_forward` so the
/// store's meta table reflects the rollback (an operator inspecting `meta`
/// after rollback sees no leftover `upgrade_step_*_to_v3` rows). The
/// per-step description is matched so a future multi-step plan's audit
/// rows are wiped in reverse order.
fn run_upgrade_step_reverse(
    transaction: &Transaction,
    from_version: u8,
    to_version: u8,
    forward_index: usize,
    _step: &crate::recovery::UpgradeStep,
) -> Result<()> {
    let audit_key = format!("upgrade_step_{forward_index}_to_v{to_version}");
    transaction
        .execute("DELETE FROM meta WHERE key=?1", params![audit_key])
        .map_err(database_error)?;
    // Reference from_version to silence dead-code warnings on a future
    // genuine migration that needs the target version to undo a DDL change.
    let _ = from_version;
    Ok(())
}

// Client identity intentionally stays out of this check: contexts are minted
// only by this store (bootstrap or registered client) and the mutation
// transaction verifies registration against the clients table, so a context
// can carry any registered client while owner/actor remain marker-bound.
fn validate_context(marker: &StoreMarker, context: &TrustedContext) -> Result<()> {
    if context.store_uuid != marker.store_uuid
        || context.owner_id != marker.owner_id
        || context.actor_id != marker.actor_id
    {
        return Err(SemanticError::MissingDependency(
            "trusted context belongs to another store".to_owned(),
        ));
    }
    Ok(())
}

fn validate_client_label(label: &str) -> Result<()> {
    let valid_bytes = label.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
    });
    if label.is_empty() || label.len() > 64 || !valid_bytes || label.starts_with("__") {
        return Err(SemanticError::MissingDependency(
            "client label must be 1-64 bytes of [a-z0-9._-] and must not start with __".to_owned(),
        ));
    }
    Ok(())
}

fn validate_capture_limits(command: &CaptureCommand, max_object_bytes: u64) -> Result<()> {
    if command.bytes.len() as u64 > max_object_bytes {
        return Err(SemanticError::InvalidCapture(format!(
            "captured object exceeds max_object_bytes={max_object_bytes}"
        )));
    }
    validate_media_type(&command.media_type)
}

fn validate_media_type(value: &str) -> Result<()> {
    fn token(part: &str) -> bool {
        !part.is_empty()
            && part.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'+' | b'.')
            })
    }
    let essence = value.split(';').next().unwrap_or("").trim();
    let valid = essence
        .split_once('/')
        .is_some_and(|(kind, subtype)| token(kind) && token(subtype));
    if !valid {
        return Err(SemanticError::InvalidCapture(
            "media_type must be a lowercase type/subtype MIME essence".to_owned(),
        ));
    }
    Ok(())
}

fn validate_object_limit(config: &SemanticConfig) -> Result<()> {
    if config.max_object_bytes == 0 {
        return Err(SemanticError::InvalidRoot(
            "max_object_bytes must be positive".to_owned(),
        ));
    }
    Ok(())
}

fn validate_operation_id(value: &str) -> Result<()> {
    let bytes = value.len();
    if bytes == 0 || bytes > 128 {
        return Err(SemanticError::MissingDependency(
            "operation_id must contain 1-128 UTF-8 bytes".to_owned(),
        ));
    }
    Ok(())
}

fn validate_interval(from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>) -> Result<()> {
    if from.zip(to).is_some_and(|(from, to)| from >= to) {
        return Err(SemanticError::InvalidInterval);
    }
    Ok(())
}

fn validate_claim_draft(draft: &ClaimDraft) -> Result<()> {
    validate_interval(draft.valid_from, draft.valid_to)?;
    if draft.subject.trim().is_empty()
        || draft.predicate.trim().is_empty()
        || draft.claim_kind.trim().is_empty()
        || draft.domain.trim().is_empty()
    {
        return Err(SemanticError::InvalidClaim(
            "subject, predicate, kind, and domain are required".to_owned(),
        ));
    }
    if draft.confidence_basis_points > 10_000 {
        return Err(SemanticError::InvalidClaim(
            "confidence_basis_points must be between 0 and 10000".to_owned(),
        ));
    }
    if draft.privacy_label != PrivacyLabel::LocalOnly {
        return Err(SemanticError::InvalidClaim(
            "privacy_label must remain local_only until a policy event authorizes release"
                .to_owned(),
        ));
    }
    Ok(())
}

fn request_hash<T: Serialize>(operation_type: &str, command: &T) -> Result<String> {
    #[derive(Serialize)]
    struct Request<'a, T> {
        operation_type: &'a str,
        command: &'a T,
    }
    Ok(sha256(&canonical_bytes(&Request {
        operation_type,
        command,
    })?))
}

fn next_event_seq(transaction: &Transaction<'_>, owner_id: Uuid) -> Result<u64> {
    let sequence: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(event_seq),0)+1 FROM events WHERE owner_id=?1",
            [owner_id.to_string()],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    u64::try_from(sequence)
        .map_err(|_| SemanticError::CorruptLedger("invalid next event sequence".to_owned()))
}

fn prior_event_hash(
    transaction: &Transaction<'_>,
    owner_id: Uuid,
    sequence: u64,
) -> Result<Option<String>> {
    if sequence == 1 {
        return Ok(None);
    }
    transaction
        .query_row(
            "SELECT event_hash FROM events WHERE owner_id=?1 AND event_seq=?2",
            params![owner_id.to_string(), (sequence - 1) as i64],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| SemanticError::CorruptLedger("event sequence is not contiguous".to_owned()))
        .map(Some)
}

fn calculate_event_hash(event: &EventEnvelope) -> Result<String> {
    event_hash_from_value(&serde_json::to_value(event).map_err(serialization_error)?)
}

/// Computes the frozen event-envelope hash with `event_hash` omitted.
pub fn event_hash_from_value(value: &Value) -> Result<String> {
    let mut value = value.clone();
    let object = value.as_object_mut().ok_or_else(|| {
        SemanticError::Serialization("event hash input must be a JSON object".to_owned())
    })?;
    object.remove("event_hash");
    Ok(sha256(&canonicalize_json(&value)?))
}

fn stored_outcome(
    transaction: &Transaction<'_>,
    context: &TrustedContext,
    operation_id: &str,
) -> Result<MutationOutcome> {
    let bytes: Option<Vec<u8>> = transaction
        .query_row(
            "SELECT outcome FROM operations WHERE owner_id=?1 AND client_id=?2 AND operation_id=?3",
            params![
                context.owner_id.to_string(),
                context.client_id.to_string(),
                operation_id
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?
        .flatten();
    let bytes = bytes.ok_or_else(|| SemanticError::MissingDependency(operation_id.to_owned()))?;
    serde_json::from_slice(&bytes).map_err(serialization_error)
}

struct ClaimScopeRow {
    domain: String,
    subject: String,
    predicate: String,
    superseded_by: Option<i64>,
    retracted_at: Option<i64>,
}

/// Build a `ClaimView` from a decrypted `ClaimRecord`, deriving the Task 2.2
/// ownership fields (`provenance_kind`, `origin`, `entity_id`) deterministically
/// from the claim payload. `entity_id_text` is the value read from the
/// `claim_status.entity_id` column (None only for legacy rows that pre-date
/// the entity table — none exist in production).
fn build_claim_view(
    claim: ClaimRecord,
    confirmed_event_seq: u64,
    entity_id_text: Option<&str>,
) -> ClaimView {
    let provenance_kind = claim.provenance.kind();
    let origin = match claim.provenance {
        // The owner asserted these directly (user_assertion) or they were
        // mechanically verified (mechanical) — human-authored origin.
        Provenance::UserAssertion { .. } | Provenance::Mechanical { .. } => {
            OriginClass::HumanAuthored
        }
        // Evidence ingest (external source) and inference (AI worker) are
        // agent-proposed, even after confirmation.
        Provenance::Evidence { .. } | Provenance::Inference { .. } => OriginClass::AgentProposed,
    };
    let entity_id = entity_id_text.and_then(|text| Uuid::parse_str(text).ok());
    ClaimView {
        claim_id: claim.claim_id,
        proposal_id: claim.proposal_id,
        subject: claim.subject,
        predicate: claim.predicate,
        value: claim.value,
        claim_kind: claim.claim_kind,
        status: claim.status,
        domain: claim.domain,
        confidence_basis_points: claim.confidence_basis_points,
        privacy_label: claim.privacy_label,
        valid_from: claim.valid_from,
        valid_to: claim.valid_to,
        confirmed_event_seq,
        provenance_kind: provenance_kind.to_owned(),
        origin,
        entity_id,
    }
}

fn insert_proposal_status(transaction: &Transaction<'_>, proposal_id: Uuid) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO proposal_status(proposal_id,status,claim_id) VALUES (?1,'proposed',NULL)",
            [proposal_id.to_string()],
        )
        .map_err(database_error)?;
    Ok(())
}

/// Shared confirm/supersede path: validates the proposal is still pending
/// and (if present) resolves and validates the claims it supersedes, then
/// builds the `claim_confirmed` event material. `superseded_claim_operations`
/// is empty for a plain confirm.
fn finish_confirmation(
    transaction: &Transaction<'_>,
    context: &TrustedContext,
    root: &Path,
    proposal_operation: &str,
    identity: EventIdentity,
    superseded_claim_operations: &[String],
) -> Result<MutationMaterial> {
    let proposed = stored_outcome(transaction, context, proposal_operation)?;
    let proposal_id = proposed.generated.proposal_id.ok_or_else(|| {
        SemanticError::InvalidTransition(format!(
            "operation {proposal_operation} did not propose a claim"
        ))
    })?;
    let bytes = decrypt_object(transaction, root, &proposed.event.payload.object_id)?;
    let proposal: ProposalObject = serde_json::from_slice(&bytes).map_err(serialization_error)?;
    validate_proposal_pending(transaction, proposal_id, &proposal)?;

    let mut superseded_claim_ids = Vec::with_capacity(superseded_claim_operations.len());
    for claim_operation in superseded_claim_operations {
        let confirmed = stored_outcome(transaction, context, claim_operation)?;
        let claim_id = confirmed.generated.claim_id.ok_or_else(|| {
            SemanticError::InvalidTransition(format!(
                "operation {claim_operation} did not confirm a claim"
            ))
        })?;
        validate_superseded_claim(transaction, claim_id, &proposal.draft)?;
        superseded_claim_ids.push(claim_id);
    }

    build_confirmation_material(
        transaction,
        proposal,
        proposal_id,
        identity,
        superseded_claim_ids,
    )
}

/// Owner-scoped counterpart of [`finish_confirmation`]: resolves the
/// proposal by `proposal_id` (bounded scan, see [`resolve_proposal_by_id`])
/// instead of the client-scoped `operations` idempotency cache, and takes
/// superseded claims directly by `claim_id` instead of by the confirming
/// client's own `operation_id`. This is what lets a reviewer (Console) act
/// on proposals/claims made by a *different* client (Task 5.1 Inbox).
fn finish_confirmation_by_id(
    transaction: &Transaction<'_>,
    root: &Path,
    owner_id: Uuid,
    proposal_id: Uuid,
    identity: EventIdentity,
    superseded_claim_ids: &[Uuid],
) -> Result<MutationMaterial> {
    let (proposal, _event_seq, _submitted_at) =
        resolve_proposal_by_id(transaction, root, owner_id, proposal_id)?;
    validate_proposal_pending(transaction, proposal_id, &proposal)?;

    for claim_id in superseded_claim_ids {
        validate_superseded_claim(transaction, *claim_id, &proposal.draft)?;
    }

    build_confirmation_material(
        transaction,
        proposal,
        proposal_id,
        identity,
        superseded_claim_ids.to_vec(),
    )
}

/// Checks a proposal is still `proposed` and not an unsupported inference —
/// shared by [`finish_confirmation`] and [`finish_confirmation_by_id`].
fn validate_proposal_pending(
    transaction: &Transaction<'_>,
    proposal_id: Uuid,
    proposal: &ProposalObject,
) -> Result<()> {
    let status: String = transaction
        .query_row(
            "SELECT status FROM proposal_status WHERE proposal_id=?1",
            [proposal_id.to_string()],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    if status != "proposed" {
        return Err(SemanticError::InvalidTransition(format!(
            "proposal {proposal_id} is already {status}, cannot confirm"
        )));
    }
    if let Provenance::Inference {
        unsupported: true, ..
    } = &proposal.provenance
    {
        return Err(SemanticError::UnsupportedInference);
    }
    Ok(())
}

/// Validates that `claim_id` is a known, still-current claim in the same
/// scope as `draft`, and not already superseded/retracted — shared by
/// [`finish_confirmation`] and [`finish_confirmation_by_id`].
fn validate_superseded_claim(
    transaction: &Transaction<'_>,
    claim_id: Uuid,
    draft: &ClaimDraft,
) -> Result<()> {
    let row: Option<ClaimScopeRow> = transaction
        .query_row(
            "SELECT domain,subject,predicate,superseded_by_event_seq,retracted_at_event_seq FROM claim_status WHERE claim_id=?1",
            [claim_id.to_string()],
            |row| {
                Ok(ClaimScopeRow {
                    domain: row.get(0)?,
                    subject: row.get(1)?,
                    predicate: row.get(2)?,
                    superseded_by: row.get(3)?,
                    retracted_at: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(database_error)?;
    let Some(ClaimScopeRow {
        domain,
        subject,
        predicate,
        superseded_by,
        retracted_at,
    }) = row
    else {
        return Err(SemanticError::InvalidTransition(format!(
            "claim {claim_id} is not a known confirmed claim"
        )));
    };
    if superseded_by.is_some() || retracted_at.is_some() {
        return Err(SemanticError::InvalidTransition(format!(
            "claim {claim_id} is already superseded or retracted"
        )));
    }
    if domain != draft.domain || subject != draft.subject || predicate != draft.predicate {
        return Err(SemanticError::InvalidTransition(format!(
            "claim {claim_id} scope ({domain}/{subject}/{predicate}) does not match new claim scope ({}/{}/{})",
            draft.domain, draft.subject, draft.predicate
        )));
    }
    Ok(())
}

/// Builds the `claim_confirmed` event material and writes `proposal_status`
/// / `claim_status` rows, given an already-resolved and already-validated
/// proposal and superseded-claim set. Shared tail of [`finish_confirmation`]
/// and [`finish_confirmation_by_id`].
fn build_confirmation_material(
    transaction: &Transaction<'_>,
    proposal: ProposalObject,
    proposal_id: Uuid,
    identity: EventIdentity,
    superseded_claim_ids: Vec<Uuid>,
) -> Result<MutationMaterial> {
    let claim_id = Uuid::now_v7();
    let ClaimDraft {
        subject,
        predicate,
        value,
        claim_kind,
        domain,
        confidence_basis_points,
        privacy_label,
        valid_from,
        valid_to,
    } = proposal.draft;
    // Resolve (or lazily create) the entity this claim attaches to, inside the
    // same confirm transaction so the claim_status row is never written
    // without an entity_id. This is the point where a confirmed claim becomes
    // bound to a stable UUIDv7 identity (ADR Decision 3, Task 2.2).
    let entity_id =
        resolve_or_create_entity_in_tx(transaction, &domain, &subject, identity.event_seq)?;
    let confirmation = ConfirmationObject {
        kind: "claim_confirmation".to_owned(),
        claim: ClaimRecord {
            claim_id,
            proposal_id,
            subject: subject.clone(),
            predicate: predicate.clone(),
            value,
            claim_kind,
            status: "confirmed".to_owned(),
            domain: domain.clone(),
            confidence_basis_points,
            privacy_label,
            valid_from,
            valid_to,
            recorded_event_id: identity.event_id,
            recorded_event_seq: identity.event_seq,
            provenance: proposal.provenance,
            supersedes: superseded_claim_ids.clone(),
            retracts: Vec::new(),
        },
    };

    transaction
        .execute(
            "UPDATE proposal_status SET status='confirmed', claim_id=?2 WHERE proposal_id=?1",
            params![proposal_id.to_string(), claim_id.to_string()],
        )
        .map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO claim_status(claim_id,domain,subject,predicate,confirmed_event_seq,superseded_by_event_seq,retracted_at_event_seq,entity_id) VALUES (?1,?2,?3,?4,?5,NULL,NULL,?6)",
            params![claim_id.to_string(), domain, subject, predicate, identity.event_seq as i64, entity_id.to_string()],
        )
        .map_err(database_error)?;
    for superseded_id in &superseded_claim_ids {
        transaction
            .execute(
                "UPDATE claim_status SET superseded_by_event_seq=?2 WHERE claim_id=?1",
                params![superseded_id.to_string(), identity.event_seq as i64],
            )
            .map_err(database_error)?;
    }

    Ok(MutationMaterial {
        event_type: "claim_confirmed",
        object_bytes: canonical_bytes(&confirmation)?,
        media_type: OBJECT_MEDIA_TYPE.to_owned(),
        generated: GeneratedIds {
            claim_id: Some(claim_id),
            ..GeneratedIds::default()
        },
    })
}

/// Owner-scoped resolution of a proposal by its `proposal_id`, independent
/// of which client originally called `propose`/`propose_inference`/etc.
/// Bounded scan over `claim_proposed` events (same shape as `claim_at`'s
/// `claim_confirmed` scan by `claim_id`) — pending-proposal counts are
/// inbox-sized, not ledger-sized, so this stays cheap without a dedicated
/// index. Returns the decrypted proposal, its event_seq, and when it was
/// submitted.
fn resolve_proposal_by_id(
    connection: &Connection,
    root: &Path,
    owner_id: Uuid,
    proposal_id: Uuid,
) -> Result<(ProposalObject, u64, DateTime<Utc>)> {
    let mut statement = connection
        .prepare(
            "SELECT event_seq,event_json,object_id FROM events \
             WHERE owner_id=?1 AND event_type='claim_proposed' ORDER BY event_seq DESC",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map(params![owner_id.to_string()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error)?;
    for row in rows {
        let (event_seq, event_json, object_id) = row.map_err(database_error)?;
        let event_seq = u64::try_from(event_seq)
            .map_err(|_| SemanticError::CorruptLedger("negative event sequence".to_owned()))?;
        let proposal: ProposalObject =
            serde_json::from_slice(&decrypt_object(connection, root, &object_id)?)
                .map_err(serialization_error)?;
        if proposal.proposal_id == proposal_id {
            let envelope: EventEnvelope =
                serde_json::from_slice(&event_json).map_err(serialization_error)?;
            return Ok((proposal, event_seq, envelope.recorded_at));
        }
    }
    Err(SemanticError::MissingDependency(proposal_id.to_string()))
}

/// Decrypts a content-addressed object and returns the UTF-8 text span
/// `[byte_start, byte_end)` from it, for evidence excerpt rendering
/// ([`SemanticStore::evidence_for`]).
fn decrypt_text_span(
    connection: &Connection,
    root: &Path,
    object_id: &str,
    byte_start: u64,
    byte_end: u64,
) -> Result<String> {
    let bytes = decrypt_object(connection, root, object_id)?;
    let start = usize::try_from(byte_start)
        .map_err(|_| SemanticError::CorruptLedger("evidence span start overflow".to_owned()))?;
    let end = usize::try_from(byte_end)
        .map_err(|_| SemanticError::CorruptLedger("evidence span end overflow".to_owned()))?;
    let span = bytes
        .get(start..end)
        .ok_or_else(|| SemanticError::CorruptLedger("evidence span is out of bounds".to_owned()))?;
    std::str::from_utf8(span)
        .map(str::to_owned)
        .map_err(|_| SemanticError::CorruptLedger("evidence span is not valid UTF-8".to_owned()))
}

// =============================================================================
// Entity model helpers (Task 2.2) — all run inside the caller's transaction.
// ADR Decision 3: entity identity is a server-minted UUIDv7 that never
// encodes the subject string, slug, or provider id. Aliases are the only way
// a former subject string or external id keeps resolving after a rename/merge.
// =============================================================================

/// Resolve `(domain, subject)` to an entity_id inside the given transaction,
/// minting a new entity (and a `canonical` alias) if none exists yet. Used by
/// `finish_confirmation` so every confirmed claim is bound to a stable entity
/// in the same transaction that writes its `claim_status` row.
fn resolve_or_create_entity_in_tx(
    connection: &Connection,
    domain: &str,
    subject: &str,
    event_seq: u64,
) -> Result<Uuid> {
    if let Some(entity_id) = resolve_entity_in_tx(connection, domain, subject)? {
        return Ok(entity_id);
    }
    let entity_id = Uuid::now_v7();
    connection
        .execute(
            "INSERT INTO entities(entity_id,domain,canonical_subject,created_at) VALUES (?1,?2,?3,?4)",
            params![entity_id.to_string(), domain, subject, now_rfc3339()],
        )
        .map_err(database_error)?;
    insert_alias(
        connection,
        domain,
        subject,
        entity_id,
        "canonical",
        event_seq,
    )?;
    Ok(entity_id)
}

/// Resolve `(domain, alias)` to an entity_id by checking both the canonical
/// subject column and the `entity_aliases` table (former subjects + external
/// ids). Returns `None` if no entity has ever held this string in this domain.
fn resolve_entity_in_tx(
    connection: &Connection,
    domain: &str,
    alias: &str,
) -> Result<Option<Uuid>> {
    let row: Option<(String,)> = connection
        .query_row(
            "SELECT entity_id FROM entity_aliases WHERE domain=?1 AND alias=?2 ORDER BY aliased_at_event_seq DESC LIMIT 1",
            params![domain, alias],
            |row| Ok((row.get::<_, String>(0)?,)),
        )
        .optional()
        .map_err(database_error)?;
    match row {
        Some((text,)) => Ok(Uuid::parse_str(&text).ok()),
        None => Ok(None),
    }
}

fn insert_alias(
    connection: &Connection,
    domain: &str,
    alias: &str,
    entity_id: Uuid,
    kind: &str,
    event_seq: u64,
) -> Result<()> {
    connection
        .execute(
            "INSERT OR IGNORE INTO entity_aliases(domain,alias,entity_id,kind,aliased_at_event_seq) VALUES (?1,?2,?3,?4,?5)",
            params![domain, alias, entity_id.to_string(), kind, event_seq as i64],
        )
        .map_err(database_error)?;
    Ok(())
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// Content identity stays the plaintext hash (preserving dedup and every
/// evidence-span byte offset/quote hash computed against plaintext), but the
/// bytes written to disk are an AEAD ciphertext under a per-object DEK,
/// itself wrapped by the owner's current epoch KEK. The wrapped-key row is
/// inserted in the same transaction as the resulting event so the two can
/// never diverge on crash.
fn publish_object(root: &Path, transaction: &Transaction<'_>, bytes: &[u8]) -> Result<String> {
    let digest = sha256(bytes);
    let object_id = format!("sha256:{digest}");
    let destination = object_path(root, &object_id)?;

    // Dedup is keyed on having a *live* wrapped key, not merely on the file
    // existing: if this exact content's key was previously destroyed
    // (crypto-shred purge), the on-disk ciphertext is permanently
    // undecryptable garbage even though a file still sits at this path.
    // Treating that case as "already stored" would silently wrap a fresh
    // DEK around a file encrypted under the destroyed one, corrupting the
    // object forever with no error at write time.
    let has_live_key: bool = transaction
        .query_row(
            "SELECT 1 FROM wrapped_keys WHERE object_id=?1",
            [&object_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(database_error)?
        .is_some();

    // Encryption and the crash_at points below run unconditionally, even
    // when the destination already exists as a true dedup (content-addressed,
    // live key) and the resulting ciphertext will just be discarded below:
    // Task 0.3's crash recovery tests deliberately re-capture identical
    // content to exercise "crash mid-dedup", and an early return here would
    // make those failpoints unreachable and silently defeat that coverage.
    let epoch: i64 = transaction
        .query_row("SELECT MAX(epoch) FROM epoch_keys", [], |row| row.get(0))
        .map_err(database_error)?;
    let kek = load_epoch_key(transaction, epoch)?;

    let dek = Aes256Gcm::generate_key(&mut OsRng);
    let content_nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = Aes256Gcm::new(&dek)
        .encrypt(&content_nonce, bytes)
        .map_err(|_| SemanticError::Serialization("object encryption failed".to_owned()))?;
    let wrap_nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let wrapped_dek = kek
        .encrypt(&wrap_nonce, dek.as_slice())
        .map_err(|_| SemanticError::Serialization("key wrap failed".to_owned()))?;

    let mut envelope = Vec::with_capacity(content_nonce.len() + ciphertext.len());
    envelope.extend_from_slice(&content_nonce);
    envelope.extend_from_slice(&ciphertext);

    let temporary = root.join("staging").join(format!("{}.tmp", Uuid::now_v7()));
    write_new_file(&temporary, &envelope)?;
    crash_at("after_object_temp_flush");
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    if destination.exists() && has_live_key {
        fs::remove_file(&temporary).map_err(io_error)?;
    } else {
        // Fresh object, or the prior ciphertext's key was destroyed: replace
        // whatever is at this path so the file on disk matches the wrapped
        // key we are about to (re)write. atomic_replace uses MoveFileExW
        // with MOVEFILE_REPLACE_EXISTING on Windows (a single syscall) so
        // there is no window where the destination is briefly missing.
        atomic_replace(&temporary, &destination)?;
    }
    crash_at("after_object_rename");

    if !has_live_key {
        transaction
            .execute(
                "INSERT OR REPLACE INTO wrapped_keys(object_id,epoch,wrapped_dek,dek_nonce) VALUES (?1,?2,?3,?4)",
                params![object_id, epoch, wrapped_dek, wrap_nonce.as_slice()],
            )
            .map_err(database_error)?;
    }
    crash_at("after_wrapped_key_insert");
    Ok(object_id)
}

fn load_epoch_key(connection: &Connection, epoch: i64) -> Result<Aes256Gcm> {
    let key_material: Vec<u8> = connection
        .query_row(
            "SELECT key_material FROM epoch_keys WHERE epoch=?1",
            [epoch],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_material);
    Ok(Aes256Gcm::new(key))
}

fn object_path(root: &Path, object_id: &str) -> Result<PathBuf> {
    let digest = object_id
        .strip_prefix("sha256:")
        .ok_or_else(|| SemanticError::CorruptLedger("invalid object ID prefix".to_owned()))?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(SemanticError::CorruptLedger(
            "invalid object digest".to_owned(),
        ));
    }
    Ok(root.join("objects").join(&digest[..2]).join(digest))
}

/// Reads and decrypts an object using an already-open connection or
/// transaction. Callers already inside a mutation transaction must use this
/// directly (opening a second connection would block on SQLite's single
/// writer); callers with no open transaction should use `read_object`.
fn decrypt_object(connection: &Connection, root: &Path, object_id: &str) -> Result<Vec<u8>> {
    // Sealed check and deny check run against the SAME connection/transaction
    // as the plaintext resolution below, so a concurrent registry_denied
    // commit can never land between "not denied" and "here is the plaintext"
    // (ADR Decision 7: fail closed for denied IDs after registry_denied).
    let sealed: String = connection
        .query_row(
            "SELECT value FROM meta WHERE key='registry_sealed'",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    if sealed == "true" {
        return Err(SemanticError::RegistrySealed(
            "purge registry is not synced with a quorum of replication targets".to_owned(),
        ));
    }
    let denied: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM purge_denied_ids WHERE denied_id=?1",
            [object_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?;
    if denied.is_some() {
        return Err(SemanticError::Denied(object_id.to_owned()));
    }
    pause_read_after_deny_check_if_armed(root);

    let envelope = fs::read(object_path(root, object_id)?).map_err(io_error)?;
    if envelope.len() < AES_GCM_NONCE_LEN {
        return Err(SemanticError::ObjectUnavailable(object_id.to_owned()));
    }
    let (content_nonce, ciphertext) = envelope.split_at(AES_GCM_NONCE_LEN);
    let content_nonce = Nonce::from_slice(content_nonce);

    let row: Option<(i64, Vec<u8>, Vec<u8>)> = connection
        .query_row(
            "SELECT epoch, wrapped_dek, dek_nonce FROM wrapped_keys WHERE object_id=?1",
            [object_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(database_error)?;
    let Some((epoch, wrapped_dek, dek_nonce)) = row else {
        return Err(SemanticError::ObjectUnavailable(object_id.to_owned()));
    };
    let kek = load_epoch_key(connection, epoch)?;
    let wrap_nonce = Nonce::from_slice(&dek_nonce);
    let dek_bytes = kek
        .decrypt(wrap_nonce, wrapped_dek.as_ref())
        .map_err(|_| SemanticError::ObjectUnavailable(object_id.to_owned()))?;
    let dek = Key::<Aes256Gcm>::from_slice(&dek_bytes);
    let plaintext = Aes256Gcm::new(dek)
        .decrypt(content_nonce, ciphertext)
        .map_err(|_| SemanticError::ObjectUnavailable(object_id.to_owned()))?;

    if format!("sha256:{}", sha256(&plaintext)) != object_id {
        return Err(SemanticError::CorruptLedger(
            "object checksum mismatch".to_owned(),
        ));
    }
    Ok(plaintext)
}

fn read_object(root: &Path, object_id: &str) -> Result<Vec<u8>> {
    // Wrapped in one explicit read transaction (not autocommit) so the
    // sealed/deny checks inside decrypt_object and the plaintext
    // materialization observe the same snapshot -- otherwise a bare
    // Connection's separate implicit-autocommit statements could let a
    // concurrent registry_denied commit land in between (TOCTOU).
    let mut connection = open_connection(root)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(database_error)?;
    let bytes = decrypt_object(&transaction, root, object_id)?;
    transaction.commit().map_err(database_error)?;
    Ok(bytes)
}

/// One hash-chained purge-registry entry, both as stored locally
/// (`purge_registry_entries`) and as replicated verbatim to each
/// independent target directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegistryEntryFile {
    epoch: u64,
    denied_ids: Vec<String>,
    entry_hash: String,
    prior_entry_hash: Option<String>,
}

fn registry_entry_hash(
    epoch: u64,
    denied_ids: &[String],
    prior_entry_hash: &Option<String>,
) -> Result<String> {
    #[derive(Serialize)]
    struct Canon<'a> {
        epoch: u64,
        denied_ids: &'a [String],
        prior_entry_hash: &'a Option<String>,
    }
    Ok(sha256(&canonical_bytes(&Canon {
        epoch,
        denied_ids,
        prior_entry_hash,
    })?))
}

/// Strict majority of `target_count` independent replication targets.
fn quorum_needed(target_count: usize) -> usize {
    target_count / 2 + 1
}

fn registry_target_path(target: &Path, epoch: u64) -> PathBuf {
    target.join(format!("epoch-{epoch:020}.json"))
}

fn write_registry_target_entry(target: &Path, entry: &RegistryEntryFile) -> Result<()> {
    let bytes = canonical_bytes(entry)?;
    let temporary = target.join(format!(".tmp-{}", Uuid::now_v7()));
    write_new_file(&temporary, &bytes)?;
    atomic_replace(&temporary, &registry_target_path(target, entry.epoch))
}

fn read_registry_target_entry(target: &Path, epoch: u64) -> Option<RegistryEntryFile> {
    let bytes = fs::read(registry_target_path(target, epoch)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The highest-epoch entry present in `target`, or `None` if the target is
/// unreachable/empty/unreadable. Directory scan rather than a separate
/// "latest" pointer file, so a partial/torn pointer write can never disagree
/// with the entry files themselves.
fn registry_target_head(target: &Path) -> Option<RegistryEntryFile> {
    let entries = fs::read_dir(target).ok()?;
    let mut best: Option<RegistryEntryFile> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with("epoch-") || !name.ends_with(".json") {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_slice::<RegistryEntryFile>(&bytes) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|current| parsed.epoch > current.epoch)
        {
            best = Some(parsed);
        }
    }
    best
}

fn local_registry_head(connection: &Connection) -> Result<Option<RegistryEntryFile>> {
    let row: Option<(i64, String, String, Option<String>)> = connection
        .query_row(
            "SELECT purge_epoch, denied_ids_json, entry_hash, prior_entry_hash \
             FROM purge_registry_entries ORDER BY purge_epoch DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(database_error)?;
    row.map(|(epoch, denied_ids_json, entry_hash, prior_entry_hash)| {
        let denied_ids: Vec<String> =
            serde_json::from_str(&denied_ids_json).map_err(serialization_error)?;
        Ok(RegistryEntryFile {
            epoch: u64::try_from(epoch)
                .map_err(|_| SemanticError::CorruptLedger("negative purge epoch".to_owned()))?,
            denied_ids,
            entry_hash,
            prior_entry_hash,
        })
    })
    .transpose()
}

fn insert_registry_entry(transaction: &Transaction<'_>, entry: &RegistryEntryFile) -> Result<()> {
    let denied_ids_json = serde_json::to_string(&entry.denied_ids).map_err(serialization_error)?;
    transaction
        .execute(
            "INSERT INTO purge_registry_entries(purge_epoch,denied_ids_json,entry_hash,prior_entry_hash,recorded_at) VALUES (?1,?2,?3,?4,?5)",
            params![
                entry.epoch as i64,
                denied_ids_json,
                entry.entry_hash,
                entry.prior_entry_hash,
                Utc::now().to_rfc3339()
            ],
        )
        .map_err(database_error)?;
    for id in &entry.denied_ids {
        transaction
            .execute(
                "INSERT OR REPLACE INTO purge_denied_ids(denied_id,purge_epoch) VALUES (?1,?2)",
                params![id, entry.epoch as i64],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

fn set_registry_sealed(connection: &Connection, sealed: bool) -> Result<()> {
    connection
        .execute(
            "UPDATE meta SET value=?1 WHERE key='registry_sealed'",
            [if sealed { "true" } else { "false" }],
        )
        .map_err(database_error)?;
    Ok(())
}

/// Fail-closed evaluation used both at `open()` and by `sync_purge_registry`:
/// no targets configured means nothing to seal over; otherwise a reachable
/// quorum must both exist and agree with (or lag behind, harmlessly) the
/// local head -- any reachable target strictly ahead of local, any
/// same-epoch hash mismatch (fork/corruption), or an unreachable quorum
/// seals the store.
fn evaluate_registry_seal(connection: &Connection, targets: &[PathBuf]) -> Result<bool> {
    if targets.is_empty() {
        return Ok(false);
    }
    let local_head = local_registry_head(connection)?;
    let local_epoch = local_head.as_ref().map_or(0, |entry| entry.epoch);
    let quorum = quorum_needed(targets.len());
    let target_heads: Vec<RegistryEntryFile> = targets
        .iter()
        .filter_map(|target| registry_target_head(target))
        .collect();
    if target_heads.len() < quorum {
        return Ok(true);
    }
    let max_target_epoch = target_heads
        .iter()
        .map(|entry| entry.epoch)
        .max()
        .unwrap_or(0);
    if max_target_epoch > local_epoch {
        return Ok(true);
    }
    if max_target_epoch == local_epoch && local_epoch > 0 {
        let local_hash = local_head.as_ref().map(|entry| entry.entry_hash.clone());
        for entry in &target_heads {
            if entry.epoch == local_epoch && Some(entry.entry_hash.clone()) != local_hash {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn validate_ledger(connection: &Connection) -> Result<()> {
    let mut statement = connection
        .prepare("SELECT event_seq,event_json,event_hash FROM events ORDER BY owner_id,event_seq")
        .map_err(database_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error)?;
    let mut prior: Option<String> = None;
    for (expected, row) in (1_u64..).zip(rows) {
        let (sequence, bytes, stored_hash) = row.map_err(database_error)?;
        let sequence = u64::try_from(sequence)
            .map_err(|_| SemanticError::CorruptLedger("negative event sequence".to_owned()))?;
        if sequence != expected {
            return Err(SemanticError::CorruptLedger(
                "non-contiguous event sequence".to_owned(),
            ));
        }
        let event: EventEnvelope = serde_json::from_slice(&bytes).map_err(serialization_error)?;
        if event.event_seq != sequence
            || event.prior_event_hash != prior
            || calculate_event_hash(&event)? != stored_hash
            || event.event_hash != stored_hash
        {
            return Err(SemanticError::CorruptLedger(
                "event hash chain mismatch".to_owned(),
            ));
        }
        prior = Some(stored_hash);
    }
    Ok(())
}

fn ledger_projection(root: &Path) -> Result<ProjectionState> {
    let connection = open_connection(root)?;
    validate_ledger(&connection)?;
    let mut statement = connection
        .prepare("SELECT event_json FROM events ORDER BY owner_id,event_seq")
        .map_err(database_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .map_err(database_error)?;
    let mut hasher = Sha256::new();
    let mut count = 0_usize;
    for row in rows {
        hasher.update(row.map_err(database_error)?);
        count += 1;
    }
    Ok(ProjectionState {
        event_count: count,
        ledger_head: count as u64,
        checksum: hex::encode(hasher.finalize()),
    })
}

fn project_and_ack(root: &Path, coordinator: &RootCoordinator) -> Result<()> {
    // Callers on the mutation path already hold a maintenance read guard.
    // parking_lot readers queue behind a waiting writer, so a plain re-read
    // here would deadlock against recovery blocked on maintenance.write().
    let _maintenance = coordinator.maintenance.read_recursive();
    let _projection = coordinator.projection.lock();
    project_and_ack_without_guard(root, coordinator)
}

fn project_and_ack_without_guard(root: &Path, coordinator: &RootCoordinator) -> Result<()> {
    let projection = ledger_projection(root)?;
    let bytes = canonical_bytes(&projection)?;
    let temporary = root.join(format!("{PROJECTION_FILE}.{}.tmp", Uuid::now_v7()));
    write_new_file(&temporary, &bytes)?;
    crash_at("after_projection_temp_flush");
    let destination = root.join(PROJECTION_FILE);
    atomic_replace(&temporary, &destination)?;
    crash_at("after_snapshot_rename_before_outbox_ack");

    let mut connection = open_connection(root)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let active = ActiveTransaction::new(coordinator);
    transaction
        .execute("UPDATE outbox SET acknowledged=1 WHERE acknowledged=0", [])
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)?;
    drop(active);
    Ok(())
}

fn remove_staging_files(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root.join("staging")).map_err(io_error)? {
        let path = entry.map_err(io_error)?.path();
        if path.is_file() {
            fs::remove_file(path).map_err(io_error)?;
        }
    }
    Ok(())
}

fn remove_unreferenced_objects(root: &Path, transaction: &Transaction<'_>) -> Result<()> {
    let objects = root.join("objects");
    if !objects.exists() {
        return Ok(());
    }
    for directory in fs::read_dir(&objects).map_err(io_error)? {
        let directory = directory.map_err(io_error)?.path();
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&directory).map_err(io_error)? {
            let path = entry.map_err(io_error)?.path();
            if !path.is_file() {
                continue;
            }
            let digest = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| SemanticError::CorruptLedger("non-UTF8 object path".to_owned()))?;
            let object_id = format!("sha256:{digest}");
            let references: i64 = transaction
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE object_id=?1",
                    [&object_id],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            if references == 0 {
                fs::remove_file(path).map_err(io_error)?;
                transaction
                    .execute("DELETE FROM wrapped_keys WHERE object_id=?1", [&object_id])
                    .map_err(database_error)?;
            }
        }
    }
    Ok(())
}

fn diagnostics(root: &Path) -> Result<StoreDiagnostics> {
    let connection = open_connection(root)?;
    validate_ledger(&connection)?;
    let events: i64 = connection
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .map_err(database_error)?;
    let operations: i64 = connection
        .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
        .map_err(database_error)?;
    let outbox_pending: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM outbox WHERE acknowledged=0",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    let mut statement = connection
        .prepare("SELECT event_seq,event_json FROM events ORDER BY event_seq")
        .map_err(database_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(database_error)?;
    let mut event_sequences = Vec::new();
    let mut hasher = Sha256::new();
    for row in rows {
        let (sequence, bytes) = row.map_err(database_error)?;
        event_sequences.push(
            u64::try_from(sequence)
                .map_err(|_| SemanticError::CorruptLedger("negative event sequence".to_owned()))?,
        );
        hasher.update(bytes);
    }
    let ledger_checksum = hex::encode(hasher.finalize());
    let projection_checksum = match fs::read(root.join(PROJECTION_FILE)) {
        Ok(bytes) => {
            serde_json::from_slice::<ProjectionState>(&bytes)
                .map_err(serialization_error)?
                .checksum
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => sha256(&[]),
        Err(error) => return Err(io_error(error)),
    };
    let objects = object_count(root)?;
    Ok(StoreDiagnostics {
        events: usize::try_from(events)
            .map_err(|_| SemanticError::CorruptLedger("invalid event count".to_owned()))?,
        operations: usize::try_from(operations)
            .map_err(|_| SemanticError::CorruptLedger("invalid operation count".to_owned()))?,
        outbox_pending: usize::try_from(outbox_pending)
            .map_err(|_| SemanticError::CorruptLedger("invalid outbox count".to_owned()))?,
        objects,
        event_sequences,
        ledger_checksum,
        projection_checksum,
    })
}

fn object_count(root: &Path) -> Result<usize> {
    let mut count = 0;
    for directory in fs::read_dir(root.join("objects")).map_err(io_error)? {
        let directory = directory.map_err(io_error)?.path();
        if directory.is_dir() {
            count += fs::read_dir(directory)
                .map_err(io_error)?
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.path().is_file())
                .count();
        }
    }
    Ok(count)
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination).map_err(io_error)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both buffers are owned, NUL-terminated UTF-16 strings and remain
    // alive for the duration of the synchronous Win32 call.
    let replaced = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(feature = "semantic-test-failpoints")]
fn crash_at(name: &str) {
    if std::env::var("SEMANTIC_TEST_FAILPOINT").as_deref() == Ok(name) {
        std::process::abort();
    }
}

#[cfg(not(feature = "semantic-test-failpoints"))]
fn crash_at(_name: &str) {}

#[cfg(feature = "semantic-test-failpoints")]
fn pause_read_after_deny_check_if_armed(root: &Path) {
    let coordinator = coordinator_for(root);
    let mut flags = coordinator.pause_read_after_deny_check.flags.lock();
    if !flags.armed {
        return;
    }
    flags.entered = true;
    coordinator.pause_read_after_deny_check.condvar.notify_all();
    while !flags.released {
        coordinator
            .pause_read_after_deny_check
            .condvar
            .wait(&mut flags);
    }
}

#[cfg(not(feature = "semantic-test-failpoints"))]
fn pause_read_after_deny_check_if_armed(_root: &Path) {}
