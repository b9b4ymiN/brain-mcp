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

use chrono::{DateTime, Utc};
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
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_parent: None,
            clock: Arc::new(SystemClock),
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
        }
    }

    /// Replaces the system clock; intended for deterministic application tests.
    pub fn with_clock(mut self, clock: Arc<dyn SemanticClock>) -> Self {
        self.clock = clock;
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
    InvalidClaim(String),
    MissingDependency(String),
    CorruptLedger(String),
    Io(String),
    Database(String),
    DatabaseContention(String),
    Serialization(String),
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
            Self::InvalidClaim(reason) => write!(formatter, "invalid claim: {reason}"),
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

#[derive(Clone, Debug)]
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
}

impl RootCoordinator {
    fn new() -> Self {
        Self {
            maintenance: RwLock::new(()),
            writer: Mutex::new(()),
            projection: Mutex::new(()),
            active_handles: AtomicUsize::new(0),
            active_transactions: AtomicUsize::new(0),
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
        initialize_schema(&connection, &marker)?;
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
        };
        Ok((store, admin))
    }

    pub fn open(root: impl AsRef<Path>, config: SemanticConfig) -> Result<Self> {
        let (_, allowed_parent) = validate_requested_root(root.as_ref(), &config)?;
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
        drop(connection);
        coordinator.active_handles.fetch_add(1, Ordering::SeqCst);
        drop(_writer);
        drop(_maintenance);
        Ok(Self {
            root: canonical_root,
            marker,
            coordinator,
            clock: config.clock,
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

    pub fn capture(
        &self,
        context: &TrustedContext,
        command: CaptureCommand,
    ) -> Result<MutationOutcome> {
        let request_hash = request_hash("capture", &command)?;
        let media_type = command.media_type.clone();
        let bytes = command.bytes.clone();
        self.mutate(
            context,
            &command.operation_id,
            &request_hash,
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
            move |transaction, _identity| {
                let captured = stored_outcome(transaction, context, &capture_operation)?;
                let proposal_id = Uuid::now_v7();
                let source_object_id = captured.event.payload.object_id;
                let source_bytes = read_object(&root, &source_object_id)?;
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
            move |transaction, identity| {
                let proposed = stored_outcome(transaction, context, &proposal_operation)?;
                let bytes = read_object(&root, &proposed.event.payload.object_id)?;
                let proposal: ProposalObject =
                    serde_json::from_slice(&bytes).map_err(serialization_error)?;
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
                        subject,
                        predicate,
                        value,
                        claim_kind,
                        status: "confirmed".to_owned(),
                        domain,
                        confidence_basis_points,
                        privacy_label,
                        valid_from,
                        valid_to,
                        recorded_event_id: identity.event_id,
                        recorded_event_seq: identity.event_seq,
                        provenance: proposal.provenance,
                        supersedes: Vec::new(),
                        retracts: Vec::new(),
                    },
                };
                Ok(MutationMaterial {
                    event_type: "claim_confirmed",
                    object_bytes: canonical_bytes(&confirmation)?,
                    media_type: OBJECT_MEDIA_TYPE.to_owned(),
                    generated: GeneratedIds {
                        claim_id: Some(claim_id),
                        ..GeneratedIds::default()
                    },
                })
            },
        )
    }

    fn mutate<F>(
        &self,
        context: &TrustedContext,
        operation_id: &str,
        request_hash: &str,
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
            match self.mutate_once(context, operation_id, request_hash, &build) {
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
        let object_id = publish_object(&self.root, &material.object_bytes)?;
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
        let connection = open_connection(&self.root)?;
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
                serde_json::from_slice(&read_object(&self.root, &object_id)?)
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

    pub fn recover(&self, _request: ManualRecovery) -> Result<()> {
        let _maintenance = self.coordinator.maintenance.write();
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

fn initialize_schema(connection: &Connection, marker: &StoreMarker) -> Result<()> {
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
             COMMIT;",
        )
        .map_err(database_error)?;
    for (key, value) in [
        ("store_uuid", marker.store_uuid.to_string()),
        ("owner_id", marker.owner_id.to_string()),
        ("schema_version", marker.schema_version.to_string()),
    ] {
        connection
            .execute(
                "INSERT INTO meta(key,value) VALUES (?1,?2)",
                params![key, value],
            )
            .map_err(database_error)?;
    }
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

fn validate_context(marker: &StoreMarker, context: &TrustedContext) -> Result<()> {
    if context.store_uuid != marker.store_uuid
        || context.owner_id != marker.owner_id
        || context.actor_id != marker.actor_id
        || context.client_id != marker.client_id
    {
        return Err(SemanticError::MissingDependency(
            "trusted context belongs to another store".to_owned(),
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

fn publish_object(root: &Path, bytes: &[u8]) -> Result<String> {
    let digest = sha256(bytes);
    let object_id = format!("sha256:{digest}");
    let temporary = root.join("staging").join(format!("{}.tmp", Uuid::now_v7()));
    write_new_file(&temporary, bytes)?;
    crash_at("after_object_temp_flush");
    let destination = object_path(root, &object_id)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    if destination.exists() {
        fs::remove_file(&temporary).map_err(io_error)?;
    } else {
        fs::rename(&temporary, &destination).map_err(io_error)?;
    }
    crash_at("after_object_rename");
    Ok(object_id)
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

fn read_object(root: &Path, object_id: &str) -> Result<Vec<u8>> {
    let bytes = fs::read(object_path(root, object_id)?).map_err(io_error)?;
    if format!("sha256:{}", sha256(&bytes)) != object_id {
        return Err(SemanticError::CorruptLedger(
            "object checksum mismatch".to_owned(),
        ));
    }
    Ok(bytes)
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
    let _maintenance = coordinator.maintenance.read();
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
