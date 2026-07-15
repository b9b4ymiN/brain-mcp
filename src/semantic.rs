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

const DATABASE_FILE: &str = "semantic.sqlite3";
const MARKER_FILE: &str = "store.marker.json";
const PROJECTION_FILE: &str = "projection.json";
const OBJECT_MEDIA_TYPE: &str = "application/vnd.brain.semantic+json";
const DEFAULT_MAX_OBJECT_BYTES: u64 = 32 * 1024 * 1024;
const AES_GCM_NONCE_LEN: usize = 12;
const BOOTSTRAP_CLIENT_LABEL: &str = "__bootstrap__";
/// Capability granted to the bootstrap identity and to any client created
/// via the unscoped `register_client`, preserving the pre-Task-1.3 behavior
/// that every registered client could confirm/reject/retract/supersede.
/// A restricted (e.g. AI extraction worker, ADR Decision 8) identity must
/// be created via `register_client_scoped` with a narrower list instead.
const DEFAULT_CLIENT_CAPABILITIES: &[&str] = &["confirm", "purge"];
const VALID_CLIENT_CAPABILITIES: &[&str] = &["confirm", "purge"];

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
            schema_version: 1,
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
            #[cfg(feature = "semantic-test-failpoints")]
            pause_after_commit: Arc::default(),
        };
        Ok((store, admin))
    }

    pub fn open(root: impl AsRef<Path>, config: SemanticConfig) -> Result<Self> {
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
        validate_database_identity(&connection, &marker)?;
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
            None,
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
            schema_version: 1,
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
                return Ok(Some(ClaimView {
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
                    confirmed_event_seq: event_seq,
                }));
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
                "SELECT claim_id, confirmed_event_seq, superseded_by_event_seq, retracted_at_event_seq
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
                    ))
                },
            )
            .map_err(database_error)?;

        let mut result = CurrentClaims::default();
        for row in rows {
            let (claim_id, confirmed_event_seq, superseded_by, retracted_at) =
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
            let view = ClaimView {
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
            };

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
        validate_database_identity(&connection, &marker)?;
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
               retracted_at_event_seq INTEGER
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

fn validate_database_identity(connection: &Connection, marker: &StoreMarker) -> Result<()> {
    let value: String = connection
        .query_row("SELECT value FROM meta WHERE key='store_uuid'", [], |row| {
            row.get(0)
        })
        .map_err(database_error)?;
    if value != marker.store_uuid.to_string() {
        return Err(SemanticError::MarkerMismatch);
    }
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

    let mut superseded_claim_ids = Vec::with_capacity(superseded_claim_operations.len());
    for claim_operation in superseded_claim_operations {
        let confirmed = stored_outcome(transaction, context, claim_operation)?;
        let claim_id = confirmed.generated.claim_id.ok_or_else(|| {
            SemanticError::InvalidTransition(format!(
                "operation {claim_operation} did not confirm a claim"
            ))
        })?;
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
        if domain != proposal.draft.domain
            || subject != proposal.draft.subject
            || predicate != proposal.draft.predicate
        {
            return Err(SemanticError::InvalidTransition(format!(
                "claim {claim_id} scope ({domain}/{subject}/{predicate}) does not match new claim scope ({}/{}/{})",
                proposal.draft.domain, proposal.draft.subject, proposal.draft.predicate
            )));
        }
        superseded_claim_ids.push(claim_id);
    }

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
    let confirmation = ConfirmationObject {
        kind: "claim_confirmation".to_owned(),
        claim: ClaimRecord {
            claim_id,
            proposal_id: proposal.proposal_id,
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
            "INSERT INTO claim_status(claim_id,domain,subject,predicate,confirmed_event_seq,superseded_by_event_seq,retracted_at_event_seq) VALUES (?1,?2,?3,?4,?5,NULL,NULL)",
            params![claim_id.to_string(), domain, subject, predicate, identity.event_seq as i64],
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
