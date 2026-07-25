use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::Serialize;
use tantivy::{
    Searcher, Term,
    query::TermQuery,
    schema::{IndexRecordOption, Value},
};

use crate::config;
use crate::engine::{EngineState, WikiEngine};
use crate::frontmatter;
use crate::git;
use crate::index_manager::UpdateReport;
use crate::index_schema::IndexSchema;
use crate::markdown;
use crate::slug::{ReadTarget, Slug, WikiUri, resolve_read_target};

/// A page that links to a given target — slug and display title.
#[derive(Debug, Clone, Serialize)]
pub struct BacklinkRef {
    /// Slug of the linking page.
    pub slug: String,
    /// Title of the linking page.
    pub title: String,
}

/// Query the index for all pages that contain a link to `target_slug`.
pub fn backlinks_query(
    searcher: &Searcher,
    is: &IndexSchema,
    target_slug: &str,
) -> Result<Vec<BacklinkRef>> {
    let f_body_links = is.field("body_links");
    let f_slug = is.field("slug");
    let f_title = is.field("title");

    let term = Term::from_field_text(f_body_links, target_slug);
    let query = TermQuery::new(term, IndexRecordOption::Basic);

    let doc_addrs = searcher.search(&query, &tantivy::collector::DocSetCollector)?;

    let mut refs: Vec<BacklinkRef> = doc_addrs
        .into_iter()
        .filter_map(|addr| {
            let doc: tantivy::TantivyDocument = searcher.doc(addr).ok()?;
            let slug = doc
                .get_first(f_slug)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = doc
                .get_first(f_title)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if slug.is_empty() {
                None
            } else {
                Some(BacklinkRef { slug, title })
            }
        })
        .collect();

    refs.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(refs)
}

/// Return all pages linking to `target_slug` in the named wiki.
pub fn backlinks_for(
    engine: &EngineState,
    wiki_name: &str,
    target_slug: &str,
) -> Result<Vec<BacklinkRef>> {
    let space = engine.space(wiki_name)?;
    let searcher = space.index_manager.searcher()?;
    backlinks_query(&searcher, &space.index_schema, target_slug)
}

/// Result of a content read — page text, asset list, or binary asset.
pub enum ContentReadResult {
    /// Page markdown content (possibly with frontmatter stripped).
    Page(String),
    /// List of co-located asset filenames.
    Assets(Vec<String>),
    /// The resolved target is a binary file — read it directly from disk.
    Binary,
}

/// Read a wiki page or list its co-located assets.
pub fn content_read(
    engine: &EngineState,
    uri: &str,
    wiki_flag: Option<&str>,
    no_frontmatter: bool,
    list_assets: bool,
) -> Result<ContentReadResult> {
    let (entry, slug) = WikiUri::resolve(uri, wiki_flag, &engine.config)?;
    let wiki_root = engine.space(&entry.name)?.wiki_root.clone();

    if list_assets {
        let slug = resolve_read_slug_with_fallback(&slug, &wiki_root)?;
        let assets = markdown::list_assets(&slug, &wiki_root)?;
        return Ok(ContentReadResult::Assets(assets));
    }

    let slug = resolve_read_slug_with_fallback(&slug, &wiki_root)?;
    match resolve_read_target(slug.as_str(), &wiki_root)? {
        ReadTarget::Page(_) => {
            let wiki_cfg = config::load_wiki(&PathBuf::from(&entry.path)).unwrap_or_default();
            let resolved = config::resolve(&engine.config, &wiki_cfg);
            let strip = no_frontmatter || resolved.read.no_frontmatter;
            let content = markdown::read_page(&slug, &wiki_root, strip)?;
            Ok(ContentReadResult::Page(content))
        }
        ReadTarget::Asset(parent_slug, filename) => {
            let parent = Slug::try_from(parent_slug.as_str())?;
            let bytes = markdown::read_asset(&parent, &filename, &wiki_root)?;
            match String::from_utf8(bytes) {
                Ok(text) => Ok(ContentReadResult::Page(text)),
                Err(_) => Ok(ContentReadResult::Binary),
            }
        }
    }
}

/// Result of a content write operation.
pub struct WriteResult {
    /// Number of bytes written to disk.
    pub bytes_written: usize,
    /// Absolute path of the written file.
    pub path: PathBuf,
    /// Canonical slug that was written after applying content placement rules.
    pub slug: String,
    /// Git commit SHA when `commit=true`, else `None`.
    /// Empty string when `commit=true` but the file was unchanged (no commit produced).
    pub commit_sha: Option<String>,
    /// Incremental index report when `commit=true`, else `None`.
    ///
    /// Note: this is a best-effort upper bound (`updated: 1` for the file
    /// we just wrote), not a measurement of the actual index update. The
    /// real counts would require threading `manager.refresh_index`'s
    /// `UpdateReport` through the ingest pipeline, which is out of scope.
    pub index_report: Option<UpdateReport>,
}

/// Return the canonical top-level folder for a frontmatter type.
///
/// The MCP layer uses this to keep model-generated pages inside the Blueprint
/// store layout even when a client sends a bare slug such as `thai-tts`.
pub fn canonical_section_for_type(type_name: &str) -> Option<&'static str> {
    match type_name.trim().to_ascii_lowercase().as_str() {
        "profile" => Some("profile"),
        "concept" | "page" | "note" | "doc" | "knowledge" => Some("concepts"),
        "entity" | "person" | "company" | "product" | "system" | "service" => Some("entities"),
        "source" | "paper" | "reference" | "url" | "article" => Some("sources"),
        "project" => Some("projects"),
        "decision" | "adr" => Some("decisions"),
        "procedure" | "procedural" | "runbook" => Some("procedural"),
        _ => None,
    }
}

fn resolve_read_slug_with_fallback(slug: &Slug, wiki_root: &Path) -> Result<Slug> {
    if resolve_read_target(slug.as_str(), wiki_root).is_ok() || slug.as_str().contains('/') {
        return Ok(slug.clone());
    }

    for section in [
        "concepts",
        "entities",
        "sources",
        "projects",
        "decisions",
        "profile",
        "procedural",
    ] {
        let candidate = Slug::try_from(format!("{section}/{slug}").as_str())?;
        if resolve_read_target(candidate.as_str(), wiki_root).is_ok() {
            return Ok(candidate);
        }
    }

    Ok(slug.clone())
}

/// Prefix bare slugs with the canonical folder for their type.
///
/// Slugs that already contain a path segment are treated as explicit and left
/// unchanged. `wiki://` URIs preserve their wiki name while normalizing the
/// slug portion.
pub fn canonicalize_uri_for_type(uri: &str, type_name: Option<&str>) -> String {
    let Some(type_name) = type_name else {
        return uri.to_string();
    };
    let Some(section) = canonical_section_for_type(type_name) else {
        return uri.to_string();
    };

    if let Some(rest) = uri.trim().strip_prefix("wiki://") {
        let Some((wiki, slug)) = rest.split_once('/') else {
            return uri.to_string();
        };
        if slug.contains('/') {
            uri.to_string()
        } else {
            format!("wiki://{wiki}/{section}/{slug}")
        }
    } else if uri.trim().contains('/') {
        uri.to_string()
    } else {
        format!("{section}/{}", uri.trim())
    }
}

/// Infer the canonical URI for a content write from the page frontmatter.
pub fn canonicalize_uri_for_content(uri: &str, content: &str) -> String {
    let parsed = frontmatter::parse(content);
    canonicalize_uri_for_type(uri, parsed.page_type())
}

/// Write content to a wiki page identified by slug or URI.
///
/// When `commit` is true (the default at the MCP layer), the function also
/// validates frontmatter, commits the file to git, and refreshes the search
/// index — i.e. a complete durable write in one call. When `commit` is false,
/// the file is written to disk only and the caller is responsible for a later
/// `wiki_ingest` call (bulk-write escape hatch).
///
/// `redact` is forwarded to the ingest pipeline when `commit` is true and runs
/// a redaction pass on the body before validation. Ignored when `commit` is
/// false.
pub fn content_write(
    engine: &EngineState,
    manager: &WikiEngine,
    uri: &str,
    wiki_flag: Option<&str>,
    content: &str,
    commit: bool,
    redact: bool,
) -> Result<WriteResult> {
    let (entry, slug) = WikiUri::resolve(uri, wiki_flag, &engine.config)?;
    let wiki_name = entry.name.clone();
    let space = engine.space(&wiki_name)?;
    let wiki_root = space.wiki_root.clone();
    let repo_root = space.repo_root.clone();
    let path = markdown::write_page(slug.as_str(), content, &wiki_root)?;

    if !commit {
        return Ok(WriteResult {
            bytes_written: content.len(),
            path,
            slug: slug.as_str().to_string(),
            commit_sha: None,
            index_report: None,
        });
    }

    // commit=true: validate + commit + index via the ingest pipeline.
    // ingest::ingest joins non-absolute paths onto wiki_root, so the path we
    // hand it must be relative to wiki_root (not repo_root) — otherwise we'd
    // get a doubled `wiki/wiki/...` prefix.
    let rel = path
        .strip_prefix(&wiki_root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| slug.as_str().to_string());
    let report = crate::ops::ingest_with_redact(
        engine, manager, &rel, /* dry_run */ false, redact, &wiki_name,
    )?;

    // ingest commits only when wiki.toml's ingest.auto_commit == true.
    // When it doesn't, force a commit so the contract holds: commit=true
    // means a durable write, period.
    let commit_sha = if report.commit.is_empty() {
        commit_with_fallback(&repo_root, &path, slug.as_str())?
    } else {
        report.commit.clone()
    };

    // ingest already called manager.refresh_index(wiki_name) internally
    // (src/ops/ingest.rs:77), so the index is current at this point. We
    // don't re-measure here — re-reading the post-commit counts would
    // require either calling refresh_index again (redundant) or threading
    // the UpdateReport through ingest_with_redact's IngestReport (out of
    // scope per the spec's non-goal on `src/ops/ingest.rs`). Instead,
    // synthesize a best-effort upper bound: we wrote one file, so the
    // index can have updated at most one page.
    let index_report = Some(UpdateReport {
        updated: 1, // best-effort upper bound; not measured
        deleted: 0,
    });

    Ok(WriteResult {
        bytes_written: content.len(),
        path,
        slug: slug.as_str().to_string(),
        commit_sha: Some(commit_sha),
        index_report,
    })
}

/// Force-commit a single path when ingest's auto_commit is disabled.
/// Returns the commit oid, or empty string when the file was unchanged.
fn commit_with_fallback(repo_root: &Path, path: &Path, slug: &str) -> Result<String> {
    let message = format!("content_write: {slug}");
    git::commit_paths(repo_root, &[path], &message)
}

/// Result of creating a new wiki page or section.
pub struct ContentNewResult {
    /// `wiki://` URI for the created page.
    pub uri: String,
    /// Slug of the created page.
    pub slug: String,
    /// Absolute filesystem path of the created file.
    pub path: PathBuf,
    /// Absolute path to the wiki root directory.
    pub wiki_root: PathBuf,
    /// True if the page was created as a bundle (folder + index.md).
    pub bundle: bool,
}

/// Create a new wiki page or section with scaffolded frontmatter.
pub fn content_new(
    engine: &EngineState,
    uri: &str,
    wiki_flag: Option<&str>,
    section: bool,
    bundle: bool,
    name: Option<&str>,
    type_: Option<&str>,
) -> Result<ContentNewResult> {
    let (entry, slug) = WikiUri::resolve(uri, wiki_flag, &engine.config)?;
    let repo_root = PathBuf::from(&entry.path);
    let wiki_root = engine.space(&entry.name)?.wiki_root.clone();

    let type_name = if section {
        "section"
    } else {
        type_.unwrap_or("page")
    };
    let body_template = resolve_body_template(&repo_root, type_name);

    let path = if section {
        markdown::create_section(&slug, &wiki_root, body_template.as_deref())?
    } else {
        markdown::create_page(
            &slug,
            bundle,
            &wiki_root,
            name,
            type_,
            body_template.as_deref(),
        )?
    };

    Ok(ContentNewResult {
        uri: format!("wiki://{}/{slug}", entry.name),
        slug: slug.as_str().to_string(),
        path,
        wiki_root,
        bundle,
    })
}

/// Resolve a body template for a type.
/// 1. `schemas/<type>.md` in the wiki repo
/// 2. Embedded default template
/// 3. None
fn resolve_body_template(repo_root: &Path, type_name: &str) -> Option<String> {
    let template_path = repo_root.join("schemas").join(format!("{type_name}.md"));
    if template_path.is_file() {
        return std::fs::read_to_string(&template_path).ok();
    }
    crate::default_schemas::embedded_body_template(type_name).map(|s| s.to_string())
}

/// Commit specified slugs (or all uncommitted files) to git and return the commit hash.
pub fn content_commit(
    engine: &EngineState,
    wiki_name: &str,
    slugs: &[String],
    all: bool,
    message: Option<&str>,
) -> Result<String> {
    let space = engine.space(wiki_name)?;

    if slugs.is_empty() && !all {
        bail!("specify slugs or --all");
    }

    if all {
        let msg = message.unwrap_or("commit: all");
        return git::commit(&space.repo_root, msg);
    }

    let mut paths = Vec::new();
    let mut committed_slugs = Vec::new();
    for s in slugs {
        let (canonical_slug, extra_paths) = rehome_bare_slug_for_commit(s, &space.wiki_root)?;
        paths.extend(extra_paths);
        committed_slugs.push(canonical_slug.clone());
        let slug = Slug::try_from(canonical_slug.as_str())?;
        let resolved = slug.resolve(&space.wiki_root)?;
        if resolved.file_name() == Some(std::ffi::OsStr::new("index.md")) {
            let bundle_dir = resolved.parent().unwrap();
            for entry in walkdir::WalkDir::new(bundle_dir)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.path().is_file() {
                    paths.push(entry.path().to_path_buf());
                }
            }
        } else {
            paths.push(resolved);
        }
    }
    let path_refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();
    let default_msg = format!("commit: {}", committed_slugs.join(", "));
    let msg = message.unwrap_or(&default_msg);
    git::commit_paths(&space.repo_root, &path_refs, msg)
}

fn rehome_bare_slug_for_commit(slug: &str, wiki_root: &Path) -> Result<(String, Vec<PathBuf>)> {
    if slug.contains('/') || slug.trim().starts_with("wiki://") {
        return Ok((slug.to_string(), vec![]));
    }

    let old_path = wiki_root.join(format!("{slug}.md"));
    if !old_path.is_file() {
        return Ok((slug.to_string(), vec![]));
    }

    let content = std::fs::read_to_string(&old_path)?;
    let parsed = frontmatter::parse(&content);
    let canonical = canonicalize_uri_for_type(slug, parsed.page_type());
    if canonical == slug {
        return Ok((slug.to_string(), vec![]));
    }

    let new_path = wiki_root.join(format!("{canonical}.md"));
    if new_path.is_file() {
        let existing = std::fs::read_to_string(&new_path)?;
        if existing == content {
            std::fs::remove_file(&old_path)?;
            return Ok((canonical, vec![old_path]));
        }
        bail!(
            "bare slug \"{slug}\" belongs under \"{canonical}\", but both files exist; resolve the duplicate manually"
        );
    }

    if let Some(parent) = new_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&old_path, &new_path)?;
    Ok((canonical, vec![old_path]))
}
