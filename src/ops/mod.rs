/// Recent activity feed — git page events projected to `ActivityEvent[]`.
pub mod activity;
mod config;
/// Read-only projection of `GlobalConfig` for the `/config` endpoint.
pub mod config_view;
mod content;
/// Wiki export operations — llms.txt, llms-full, and JSON export formats.
pub mod export;
mod graph;
mod history;
mod index;
mod ingest;
mod lint;
mod logs;
/// Redaction pass — strip PII and confidential values from page bodies.
pub mod redact;
mod schema;
mod search;
mod spaces;
mod stats;
mod suggest;

pub use activity::*;
pub use config::*;
pub use content::*;
pub use export::*;
pub use graph::*;
pub use history::*;
pub use index::*;
pub use ingest::*;
pub use lint::*;
pub use logs::*;
pub use redact::*;
pub use schema::*;
pub use search::*;
pub use spaces::*;
pub use stats::*;
pub use suggest::*;
