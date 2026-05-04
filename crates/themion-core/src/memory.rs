use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, ToSql};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "semantic-memory")]
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "semantic-memory")]
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const DEFAULT_SEARCH_LIMIT: u32 = 20;
const MAX_SEARCH_LIMIT: u32 = 100;
const DEFAULT_GRAPH_DEPTH: u32 = 1;
const MAX_GRAPH_DEPTH: u32 = 3;
const DEFAULT_GRAPH_LIMIT: u32 = 50;
const MAX_GRAPH_LIMIT: u32 = 200;
pub const GLOBAL_PROJECT_DIR: &str = "[GLOBAL]";

pub const UNIFIED_SEARCH_CHUNKING_VERSION: &str = "v1-char-1200-overlap-200";
#[cfg(feature = "semantic-memory")]
const UNIFIED_SEARCH_CHUNK_LEN: usize = 1200;
#[cfg(feature = "semantic-memory")]
const UNIFIED_SEARCH_CHUNK_OVERLAP: usize = 200;

const UNIFIED_SEARCH_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS unified_search_documents (
    document_id TEXT PRIMARY KEY,
    source_kind TEXT NOT NULL,
    source_id TEXT NOT NULL,
    project_dir TEXT NOT NULL,
    session_id TEXT,
    turn_seq INTEGER,
    tool_call_id TEXT,
    title TEXT NOT NULL,
    source_text TEXT NOT NULL,
    source_updated_at_ms INTEGER NOT NULL,
    chunking_version TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    embedding_state TEXT NOT NULL,
    last_indexed_at_ms INTEGER,
    last_error TEXT,
    UNIQUE(source_kind, source_id, project_dir)
);
CREATE INDEX IF NOT EXISTS idx_unified_search_documents_scope_state
    ON unified_search_documents(project_dir, source_kind, embedding_state, source_updated_at_ms DESC);

CREATE TABLE IF NOT EXISTS unified_search_chunks (
    chunk_id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES unified_search_documents(document_id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    char_start INTEGER NOT NULL,
    char_len INTEGER NOT NULL,
    chunk_text TEXT NOT NULL,
    token_start INTEGER,
    token_len INTEGER,
    embedding_model TEXT NOT NULL,
    embedding_dim INTEGER NOT NULL,
    embedding_blob BLOB NOT NULL,
    source_updated_at_ms INTEGER NOT NULL,
    indexed_at_ms INTEGER NOT NULL,
    UNIQUE(document_id, embedding_model, chunk_index)
);
CREATE INDEX IF NOT EXISTS idx_unified_search_chunks_document
    ON unified_search_chunks(document_id, chunk_index);

";

#[cfg(feature = "semantic-memory")]
const DEFAULT_SEMANTIC_MODEL: &str = "bge-micro-v2";
fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(feature = "semantic-memory")]
fn semantic_cache_dir() -> Result<PathBuf> {
    let data_dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
        })
        .ok_or_else(|| anyhow::anyhow!("no data dir available for semantic-memory cache"))?;
    Ok(data_dir.join("themion").join("fastembed"))
}

#[cfg(feature = "semantic-memory")]
fn configure_fastembed_cache_dir() -> Result<()> {
    let cache_dir = semantic_cache_dir()?;
    std::fs::create_dir_all(&cache_dir)?;
    std::env::set_var("FASTEMBED_CACHE_DIR", &cache_dir);
    Ok(())
}

const MEMORY_SCHEMA_BASE: &str = "
CREATE TABLE IF NOT EXISTS memory_nodes (
    node_id TEXT PRIMARY KEY,
    project_dir TEXT NOT NULL DEFAULT '[GLOBAL]',
    node_type TEXT NOT NULL,
    title TEXT NOT NULL,
    content TEXT,
    metadata_json TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_nodes_project_type_updated
    ON memory_nodes(project_dir, node_type, updated_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_memory_nodes_type_updated
    ON memory_nodes(node_type, updated_at_ms DESC);

CREATE TABLE IF NOT EXISTS memory_node_hashtags (
    node_id TEXT NOT NULL REFERENCES memory_nodes(node_id) ON DELETE CASCADE,
    hashtag TEXT NOT NULL,
    PRIMARY KEY (node_id, hashtag)
);
CREATE INDEX IF NOT EXISTS idx_memory_node_hashtags_tag
    ON memory_node_hashtags(hashtag, node_id);

CREATE TABLE IF NOT EXISTS memory_edges (
    edge_id TEXT PRIMARY KEY,
    from_node_id TEXT NOT NULL REFERENCES memory_nodes(node_id) ON DELETE CASCADE,
    to_node_id TEXT NOT NULL REFERENCES memory_nodes(node_id) ON DELETE CASCADE,
    relation_type TEXT NOT NULL,
    metadata_json TEXT,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_edges_from
    ON memory_edges(from_node_id, relation_type, to_node_id);
CREATE INDEX IF NOT EXISTS idx_memory_edges_to
    ON memory_edges(to_node_id, relation_type, from_node_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_edges_unique
    ON memory_edges(from_node_id, to_node_id, relation_type);
";

const MEMORY_SCHEMA_FTS5: &str = "
CREATE VIRTUAL TABLE IF NOT EXISTS memory_nodes_fts USING fts5(
    title,
    content,
    content='memory_nodes',
    content_rowid='rowid',
    tokenize='porter unicode61'
);
CREATE TRIGGER IF NOT EXISTS memory_nodes_ai AFTER INSERT ON memory_nodes BEGIN
    INSERT INTO memory_nodes_fts(rowid, title, content) VALUES (new.rowid, new.title, new.content);
END;
CREATE TRIGGER IF NOT EXISTS memory_nodes_ad AFTER DELETE ON memory_nodes BEGIN
    INSERT INTO memory_nodes_fts(memory_nodes_fts, rowid, title, content)
    VALUES('delete', old.rowid, old.title, old.content);
END;
CREATE TRIGGER IF NOT EXISTS memory_nodes_au AFTER UPDATE ON memory_nodes BEGIN
    INSERT INTO memory_nodes_fts(memory_nodes_fts, rowid, title, content)
    VALUES('delete', old.rowid, old.title, old.content);
    INSERT INTO memory_nodes_fts(rowid, title, content) VALUES (new.rowid, new.title, new.content);
END;
";

fn migrate_project_dir(conn: &Connection) -> Result<()> {
    let has_project_dir = conn
        .prepare("PRAGMA table_info(memory_nodes)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "project_dir");
    if !has_project_dir {
        conn.execute(
            "ALTER TABLE memory_nodes ADD COLUMN project_dir TEXT NOT NULL DEFAULT '[GLOBAL]'",
            [],
        )?;
    }
    conn.execute(
        "UPDATE memory_nodes SET project_dir = ?1 WHERE project_dir = ''",
        params![GLOBAL_PROJECT_DIR],
    )?;
    Ok(())
}

fn drop_legacy_semantic_memory_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS memory_node_embeddings;
         DROP TABLE IF EXISTS memory_embedding_queue;",
    )?;
    Ok(())
}

pub fn init_schema(conn: &Connection, fts5: bool) -> Result<()> {
    conn.execute_batch(MEMORY_SCHEMA_BASE)?;
    migrate_project_dir(conn)?;
    if fts5 {
        conn.execute_batch(MEMORY_SCHEMA_FTS5)?;
    }
    conn.execute_batch(UNIFIED_SEARCH_SCHEMA)?;
    drop_legacy_semantic_memory_tables(conn)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryNode {
    pub node_id: String,
    pub project_dir: String,
    pub node_type: String,
    pub title: String,
    pub content: Option<String>,
    pub hashtags: Vec<String>,
    pub metadata_json: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryEdge {
    pub edge_id: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub relation_type: String,
    pub metadata_json: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryNodeWithLinks {
    #[serde(flatten)]
    pub node: MemoryNode,
    pub outgoing: Vec<MemoryEdge>,
    pub incoming: Vec<MemoryEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryGraph {
    pub nodes: Vec<MemoryNode>,
    pub edges: Vec<MemoryEdge>,
    pub truncated: bool,
    pub depth: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryHashtagStat {
    pub hashtag: String,
    pub node_count: u32,
}

#[derive(Debug, Clone)]
pub struct CreateNodeArgs {
    pub node_id: Option<String>,
    pub project_dir: String,
    pub node_type: String,
    pub title: String,
    pub content: Option<String>,
    pub hashtags: Vec<String>,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateNodeArgs {
    pub node_type: Option<String>,
    pub title: Option<String>,
    pub content: Option<Option<String>>,
    pub hashtags: Option<Vec<String>>,
    pub metadata_json: Option<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct LinkNodesArgs {
    pub edge_id: Option<String>,
    pub from_node_id: String,
    pub to_node_id: String,
    pub relation_type: String,
    pub metadata_json: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchNodesArgs {
    pub query: Option<String>,
    pub project_dir: String,
    pub hashtags: Vec<String>,
    pub hashtag_match: HashtagMatch,
    pub node_type: Option<String>,
    pub relation_type: Option<String>,
    pub linked_node_id: Option<String>,
    pub limit: u32,
    pub mode: UnifiedSearchMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemorySearchResponse {
    pub mode: UnifiedSearchMode,
    pub degraded: bool,
    pub degradation_reason: Option<String>,
    pub pending_index_count: u32,
    pub nodes: Vec<MemoryNode>,
}


#[derive(Debug, Clone, Serialize)]
pub struct UnifiedSearchSnippet {
    pub text: String,
    pub char_start: u32,
    pub char_len: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnifiedSearchResult {
    pub source_kind: String,
    pub source_id: String,
    pub project_dir: String,
    pub score: f64,
    pub score_kind: UnifiedSearchMode,
    pub snippet: String,
    pub primary_snippet: String,
    pub supporting_snippets: Vec<UnifiedSearchSnippet>,
    pub title: String,
    pub node_type: Option<String>,
    pub hashtags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnifiedSearchResponse {
    pub mode: UnifiedSearchMode,
    pub degraded: bool,
    pub degradation_reason: Option<String>,
    pub pending_index_count: u32,
    pub unavailable_source_kinds: Vec<String>,
    pub results: Vec<UnifiedSearchResult>,
}


#[derive(Debug, Clone, Serialize)]
pub struct UnifiedSearchIndexReport {
    pub mode: String,
    pub requested_full: bool,
    pub project_dir: Option<String>,
    pub source_kind: Option<String>,
    pub queued_before: u32,
    pub indexed_documents: u32,
    pub skipped_documents: u32,
    pub failed_documents: u32,
    pub removed_documents: u32,
    pub remaining_pending: u32,
}

#[derive(Debug, Clone)]
#[cfg(feature = "semantic-memory")]
struct UnifiedSearchDocumentInput {
    source_kind: String,
    source_id: String,
    project_dir: String,
    session_id: Option<String>,
    turn_seq: Option<u32>,
    tool_call_id: Option<String>,
    title: String,
    source_text: String,
    source_updated_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct AppendedChatMessageIndexArgs {
    pub message_id: i64,
    pub session_id: String,
    pub turn_seq: u32,
    pub role: String,
    pub content: Option<String>,
    pub tool_calls_json: Option<String>,
    pub project_dir: String,
    pub created_at_s: i64,
}

#[derive(Debug, Clone)]
#[cfg(feature = "semantic-memory")]
struct PendingUnifiedSearchDocument {
    source_kind: String,
    source_id: String,
    project_dir: String,
    session_id: Option<String>,
    turn_seq: Option<u32>,
    tool_call_id: Option<String>,
    title: String,
    source_text: String,
    source_updated_at_ms: i64,
}

#[derive(Debug, Clone)]
#[cfg(feature = "semantic-memory")]
struct UnifiedSearchChunkDraft {
    chunk_index: u32,
    char_start: u32,
    char_len: u32,
    chunk_text: String,
}

#[cfg(feature = "semantic-memory")]
#[derive(Debug, Clone, Serialize)]
pub struct MemoryIndexReport {
    pub mode: String,
    pub requested_full: bool,
    pub queued_before: u32,
    pub scanned_candidates: u32,
    pub indexed_nodes: u32,
    pub skipped_nodes: u32,
    pub removed_stale_embeddings: u32,
    pub remaining_pending: u32,
    pub failed_nodes: u32,
    pub found: Vec<MemoryIndexNodeReport>,
    pub indexed: Vec<MemoryIndexNodeReport>,
    pub skipped: Vec<MemoryIndexNodeReport>,
    pub failures: Vec<MemoryIndexFailure>,
    pub model_tag: String,
}

#[cfg(feature = "semantic-memory")]
#[derive(Debug, Clone, Serialize)]
pub struct MemoryIndexNodeReport {
    pub node_id: String,
    pub title: String,
    pub reason: String,
}

#[cfg(feature = "semantic-memory")]
#[derive(Debug, Clone, Serialize)]
pub struct MemoryIndexFailure {
    pub node_id: String,
    pub title: String,
    pub error: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UnifiedSearchMode {
    Fts,
    Semantic,
    Hybrid,
}

impl UnifiedSearchMode {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "fts" => Some(Self::Fts),
            "semantic" => Some(Self::Semantic),
            "hybrid" => Some(Self::Hybrid),
            _ => None,
        }
    }
}

impl Default for UnifiedSearchMode {
    fn default() -> Self {
        Self::Fts
    }
}

pub fn memory_search_to_unified(response: MemorySearchResponse) -> UnifiedSearchResponse {
    let score_kind = match response.mode {
        UnifiedSearchMode::Fts => UnifiedSearchMode::Fts,
        UnifiedSearchMode::Semantic => UnifiedSearchMode::Semantic,
        UnifiedSearchMode::Hybrid => UnifiedSearchMode::Hybrid,
    };
    let results = response
        .nodes
        .into_iter()
        .enumerate()
        .map(|(idx, node)| {
            let primary_snippet = node
                .content
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| node.title.clone());
            let score = match score_kind {
                UnifiedSearchMode::Fts => 1.0 - ((idx as f64) * 0.01),
                UnifiedSearchMode::Semantic => 1.0 - ((idx as f64) * 0.01),
                UnifiedSearchMode::Hybrid => 1.0 - ((idx as f64) * 0.01),
            };
            UnifiedSearchResult {
                source_kind: "memory".to_string(),
                source_id: node.node_id.clone(),
                project_dir: node.project_dir.clone(),
                score: if score < 0.05 { 0.05 } else { score },
                score_kind,
                snippet: primary_snippet.clone(),
                primary_snippet: primary_snippet.clone(),
                supporting_snippets: Vec::new(),
                title: node.title,
                node_type: Some(node.node_type),
                hashtags: node.hashtags,
            }
        })
        .collect();
    UnifiedSearchResponse {
        mode: response.mode,
        degraded: response.degraded,
        degradation_reason: response.degradation_reason,
        pending_index_count: response.pending_index_count,
        unavailable_source_kinds: Vec::new(),
        results,
    }
}

pub fn append_unified_search_rows(
    response: &mut UnifiedSearchResponse,
    rows: Vec<crate::db::UnifiedSearchSourceRow>,
    mode: UnifiedSearchMode,
) {
    let base_index = response.results.len();
    for (idx, row) in rows.into_iter().enumerate() {
        let score = match mode {
            UnifiedSearchMode::Fts => 0.30 - (((base_index + idx) as f64) * 0.01),
            UnifiedSearchMode::Semantic => 0.30 - (((base_index + idx) as f64) * 0.01),
            UnifiedSearchMode::Hybrid => 0.30 - (((base_index + idx) as f64) * 0.01),
        };
        response.results.push(UnifiedSearchResult {
            source_kind: row.source_kind,
            source_id: row.source_id,
            project_dir: row.project_dir,
            score: if score < 0.05 { 0.05 } else { score },
            score_kind: if matches!(mode, UnifiedSearchMode::Hybrid) { UnifiedSearchMode::Fts } else { mode },
            snippet: row.snippet.clone(),
            primary_snippet: row.snippet.clone(),
            supporting_snippets: Vec::new(),
            title: row.title,
            node_type: row.role,
            hashtags: Vec::new(),
        });
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HashtagMatch {
    #[default]
    Any,
    All,
}

impl HashtagMatch {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "any" => Some(Self::Any),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

pub struct MemoryStore<'a> {
    conn: &'a Mutex<Connection>,
    fts5: bool,
}

impl<'a> MemoryStore<'a> {
    pub fn new(conn: &'a Mutex<Connection>, fts5: bool) -> Self {
        Self { conn, fts5 }
    }

    pub fn create_node(&self, args: CreateNodeArgs) -> Result<MemoryNode> {
        let node_id = normalize_optional_uuid(args.node_id)?;
        let project_dir = normalize_project_dir(&args.project_dir)?;
        let node_type = normalize_required_label(&args.node_type, "node_type")?;
        let title = normalize_required_title(&args.title)?;
        let hashtags = normalize_hashtags(args.hashtags)?;
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO memory_nodes (node_id, project_dir, node_type, title, content, metadata_json, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![node_id, project_dir, node_type, title, args.content, args.metadata_json, now_ms, now_ms],
        )?;
        replace_hashtags(&conn, &node_id, &hashtags)?;
        #[cfg(feature = "semantic-memory")]
        register_memory_node_for_unified_search(&conn, &node_id, &project_dir, &title, args.content.as_deref(), now_ms)?;
        drop(conn);
        self.get_node(&node_id)?
            .ok_or_else(|| anyhow::anyhow!("memory node insert failed"))
    }

    pub fn update_node(&self, node_id: &str, args: UpdateNodeArgs) -> Result<Option<MemoryNode>> {
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        let existing: Option<(String, String, String, Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT project_dir, node_type, title, content, metadata_json FROM memory_nodes WHERE node_id = ?1",
                params![node_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()?;
        let Some((project_dir, old_type, old_title, old_content, old_metadata)) = existing else {
            return Ok(None);
        };
        let node_type = match args.node_type {
            Some(value) => normalize_required_label(&value, "node_type")?,
            None => old_type,
        };
        let title = match args.title {
            Some(value) => normalize_required_title(&value)?,
            None => old_title,
        };
        let content = args.content.unwrap_or(old_content);
        let metadata_json = args.metadata_json.unwrap_or(old_metadata);
        conn.execute(
            "UPDATE memory_nodes
             SET node_type = ?2, title = ?3, content = ?4, metadata_json = ?5, updated_at_ms = ?6
             WHERE node_id = ?1",
            params![node_id, node_type, title, content, metadata_json, now_ms],
        )?;
        if let Some(hashtags) = args.hashtags {
            let hashtags = normalize_hashtags(hashtags)?;
            replace_hashtags(&conn, node_id, &hashtags)?;
        }
        #[cfg(feature = "semantic-memory")]
        register_memory_node_for_unified_search(&conn, node_id, &project_dir, &title, content.as_deref(), now_ms)?;
        drop(conn);
        self.get_node(node_id)
    }

    pub fn delete_node(&self, node_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "DELETE FROM memory_nodes WHERE node_id = ?1",
            params![node_id],
        )?;
        Ok(changed > 0)
    }

    pub fn link_nodes(&self, args: LinkNodesArgs) -> Result<MemoryEdge> {
        let edge_id = normalize_optional_uuid(args.edge_id)?;
        let relation_type = normalize_required_label(&args.relation_type, "relation_type")?;
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        let from_exists = node_exists(&conn, &args.from_node_id)?;
        let to_exists = node_exists(&conn, &args.to_node_id)?;
        if !from_exists || !to_exists {
            anyhow::bail!("from_node_id and to_node_id must both exist");
        }
        conn.execute(
            "INSERT INTO memory_edges (edge_id, from_node_id, to_node_id, relation_type, metadata_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![edge_id, args.from_node_id, args.to_node_id, relation_type, args.metadata_json, now_ms],
        )?;
        drop(conn);
        self.get_edge(&edge_id)?
            .ok_or_else(|| anyhow::anyhow!("memory edge insert failed"))
    }

    pub fn unlink_nodes(
        &self,
        edge_id: Option<&str>,
        from_node_id: Option<&str>,
        to_node_id: Option<&str>,
        relation_type: Option<&str>,
    ) -> Result<u32> {
        let conn = self.conn.lock().unwrap();
        let changed = if let Some(edge_id) = edge_id {
            conn.execute(
                "DELETE FROM memory_edges WHERE edge_id = ?1",
                params![edge_id],
            )?
        } else {
            let from_node_id =
                from_node_id.ok_or_else(|| anyhow::anyhow!("missing from_node_id"))?;
            let to_node_id = to_node_id.ok_or_else(|| anyhow::anyhow!("missing to_node_id"))?;
            let relation_type =
                relation_type.ok_or_else(|| anyhow::anyhow!("missing relation_type"))?;
            conn.execute(
                "DELETE FROM memory_edges WHERE from_node_id = ?1 AND to_node_id = ?2 AND relation_type = ?3",
                params![from_node_id, to_node_id, relation_type],
            )?
        };
        Ok(changed as u32)
    }

    pub fn get_node_with_links(&self, node_id: &str) -> Result<Option<MemoryNodeWithLinks>> {
        let Some(node) = self.get_node(node_id)? else {
            return Ok(None);
        };
        let conn = self.conn.lock().unwrap();
        let outgoing = edges_for(&conn, "from_node_id", node_id)?;
        let incoming = edges_for(&conn, "to_node_id", node_id)?;
        Ok(Some(MemoryNodeWithLinks {
            node,
            outgoing,
            incoming,
        }))
    }

    pub fn search_nodes(&self, args: SearchNodesArgs) -> Result<MemorySearchResponse> {
        match args.mode {
            UnifiedSearchMode::Fts => self.search_nodes_fts(args),
            UnifiedSearchMode::Semantic | UnifiedSearchMode::Hybrid => Ok(MemorySearchResponse {
                mode: args.mode,
                degraded: true,
                degradation_reason: Some("direct semantic Project Memory search is retired; use unified_search for semantic or hybrid retrieval".to_string()),
                pending_index_count: 0,
                nodes: Vec::new(),
            }),
        }
    }

    fn search_nodes_fts(&self, mut args: SearchNodesArgs) -> Result<MemorySearchResponse> {
        args.limit = clamp_limit(args.limit, DEFAULT_SEARCH_LIMIT, MAX_SEARCH_LIMIT);
        args.hashtags = normalize_hashtags(args.hashtags)?;
        let conn = self.conn.lock().unwrap();
        let mut params_vec: Vec<Box<dyn ToSql>> = Vec::new();
        let mut sql = if args
            .query
            .as_ref()
            .map(|q| !q.trim().is_empty())
            .unwrap_or(false)
            && self.fts5
        {
            params_vec.push(Box::new(args.query.unwrap_or_default()));
            "SELECT DISTINCT n.node_id, n.project_dir, n.node_type, n.title, n.content, n.metadata_json, n.created_at_ms, n.updated_at_ms
             FROM memory_nodes_fts f
             JOIN memory_nodes n ON f.rowid = n.rowid
             WHERE memory_nodes_fts MATCH ?".to_string()
        } else {
            "SELECT DISTINCT n.node_id, n.project_dir, n.node_type, n.title, n.content, n.metadata_json, n.created_at_ms, n.updated_at_ms
             FROM memory_nodes n
             WHERE 1=1".to_string()
        };

        let project_dir = normalize_project_dir(&args.project_dir)?;
        sql.push_str(" AND n.project_dir = ?");
        params_vec.push(Box::new(project_dir));

        if let Some(node_type) = args.node_type.as_ref().filter(|v| !v.trim().is_empty()) {
            sql.push_str(" AND n.node_type = ?");
            params_vec.push(Box::new(normalize_required_label(node_type, "node_type")?));
        }
        if let Some(relation_type) = args.relation_type.as_ref().filter(|v| !v.trim().is_empty()) {
            let relation_type = normalize_required_label(relation_type, "relation_type")?;
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM memory_edges e
                   WHERE (e.from_node_id = n.node_id OR e.to_node_id = n.node_id)
                     AND e.relation_type = ?",
            );
            params_vec.push(Box::new(relation_type));
            if let Some(linked) = args
                .linked_node_id
                .as_ref()
                .filter(|v| !v.trim().is_empty())
            {
                sql.push_str(" AND (e.from_node_id = ? OR e.to_node_id = ?)");
                params_vec.push(Box::new(linked.clone()));
                params_vec.push(Box::new(linked.clone()));
            }
            sql.push(')');
        } else if let Some(linked) = args
            .linked_node_id
            .as_ref()
            .filter(|v| !v.trim().is_empty())
        {
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM memory_edges e
                   WHERE (e.from_node_id = n.node_id AND e.to_node_id = ?)
                      OR (e.to_node_id = n.node_id AND e.from_node_id = ?))",
            );
            params_vec.push(Box::new(linked.clone()));
            params_vec.push(Box::new(linked.clone()));
        }
        match args.hashtag_match {
            HashtagMatch::All => {
                for hashtag in &args.hashtags {
                    sql.push_str(" AND EXISTS (SELECT 1 FROM memory_node_hashtags h WHERE h.node_id = n.node_id AND h.hashtag = ?)");
                    params_vec.push(Box::new(hashtag.clone()));
                }
            }
            HashtagMatch::Any if !args.hashtags.is_empty() => {
                sql.push_str(
                    " AND n.node_id IN (SELECT node_id FROM memory_node_hashtags WHERE hashtag IN (",
                );
                for idx in 0..args.hashtags.len() {
                    if idx > 0 {
                        sql.push_str(", ");
                    }
                    sql.push('?');
                    params_vec.push(Box::new(args.hashtags[idx].clone()));
                }
                sql.push_str("))");
            }
            HashtagMatch::Any => {}
        }
        sql.push_str(" ORDER BY n.updated_at_ms DESC LIMIT ?");
        params_vec.push(Box::new(args.limit as i64));

        let params_ref: Vec<&dyn ToSql> = params_vec
            .iter()
            .map(|v| v.as_ref() as &dyn ToSql)
            .collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_ref.as_slice(), |row| {
            map_node_row_with_conn(&conn, row)
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(MemorySearchResponse {
            mode: UnifiedSearchMode::Fts,
            degraded: false,
            degradation_reason: None,
            pending_index_count: 0,
            nodes: out,
        })
    }

    #[cfg(feature = "semantic-memory")]
    pub fn index_pending_embeddings(&self, force_full: bool) -> Result<MemoryIndexReport> {
        let report = self.rebuild_unified_search_index(None, Some("memory"), force_full)?;
        Ok(MemoryIndexReport {
            mode: report.mode,
            requested_full: report.requested_full,
            queued_before: report.queued_before,
            scanned_candidates: report.queued_before,
            indexed_nodes: report.indexed_documents,
            skipped_nodes: report.skipped_documents,
            removed_stale_embeddings: report.removed_documents,
            remaining_pending: report.remaining_pending,
            failed_nodes: report.failed_documents,
            found: Vec::new(),
            indexed: Vec::new(),
            skipped: Vec::new(),
            failures: Vec::new(),
            model_tag: DEFAULT_SEMANTIC_MODEL.to_string(),
        })
    }

    #[cfg(feature = "semantic-memory")]
    pub fn register_appended_chat_message_for_unified_search(
        &self,
        args: AppendedChatMessageIndexArgs,
    ) -> Result<bool> {
        let Some(doc) = build_chat_message_document_input(
            args.message_id,
            &args.session_id,
            args.turn_seq,
            &args.role,
            args.content.as_deref(),
            args.tool_calls_json.as_deref(),
            &args.project_dir,
            args.created_at_s,
        ) else {
            return Ok(false);
        };
        let conn = self.conn.lock().unwrap();
        upsert_unified_search_document_pending(&conn, &doc)?;
        Ok(true)
    }

    #[cfg(not(feature = "semantic-memory"))]
    pub fn register_appended_chat_message_for_unified_search(
        &self,
        _args: AppendedChatMessageIndexArgs,
    ) -> Result<bool> {
        Ok(false)
    }

    pub fn drain_pending_chat_message_unified_search(
        &self,
        project_dir: &str,
        limit: u32,
    ) -> Result<UnifiedSearchIndexReport> {
        #[cfg(not(feature = "semantic-memory"))]
        {
            let _ = (project_dir, limit);
            return Ok(UnifiedSearchIndexReport {
                mode: "unified-search-index".to_string(),
                requested_full: false,
                project_dir: Some(project_dir.to_string()),
                source_kind: Some("chat_message".to_string()),
                queued_before: 0,
                indexed_documents: 0,
                skipped_documents: 0,
                failed_documents: 0,
                removed_documents: 0,
                remaining_pending: 0,
            });
        }
        #[cfg(feature = "semantic-memory")]
        {
            let project_dir = normalize_project_dir(project_dir)?;
            let conn = self.conn.lock().unwrap();
            let pending = list_pending_unified_search_documents(&conn, &project_dir, "chat_message", limit)?;
            let queued_before = pending.len() as u32;
            let mut indexed_documents = 0u32;
            let mut failed_documents = 0u32;
            for doc in pending.iter() {
                match index_pending_unified_search_document(&conn, doc) {
                    Ok(()) => indexed_documents += 1,
                    Err(_) => failed_documents += 1,
                }
            }
            let remaining_pending: u32 = conn.query_row(
                "SELECT COUNT(*) FROM unified_search_documents WHERE project_dir = ?1 AND source_kind = 'chat_message' AND embedding_state = 'pending'",
                params![project_dir],
                |row| row.get::<_, i64>(0),
            )? as u32;
            Ok(UnifiedSearchIndexReport {
                mode: "unified-search-index".to_string(),
                requested_full: false,
                project_dir: Some(project_dir),
                source_kind: Some("chat_message".to_string()),
                queued_before,
                indexed_documents,
                skipped_documents: 0,
                failed_documents,
                removed_documents: 0,
                remaining_pending,
            })
        }
    }

    pub fn rebuild_unified_search_index(
        &self,
        project_dir: Option<&str>,
        source_kind: Option<&str>,
        full: bool,
    ) -> Result<UnifiedSearchIndexReport> {
        #[cfg(not(feature = "semantic-memory"))]
        {
            let _ = (project_dir, source_kind, full);
            Ok(UnifiedSearchIndexReport {
                mode: "unified-search-index".to_string(),
                requested_full: full,
                project_dir: project_dir.map(str::to_string),
                source_kind: source_kind.map(str::to_string),
                queued_before: 0,
                indexed_documents: 0,
                skipped_documents: 0,
                failed_documents: 0,
                removed_documents: 0,
                remaining_pending: 0,
            })
        }
        #[cfg(feature = "semantic-memory")]
        {
            let conn = self.conn.lock().unwrap();
            let requested_project_dir = match project_dir {
                Some(value) => Some(normalize_project_dir(value)?),
                None => None,
            };
            let requested_source_kind = source_kind.map(|value| value.trim().to_string());
            if full {
                clear_unified_search_scope(&conn, requested_project_dir.as_deref(), requested_source_kind.as_deref())?;
            }
            let docs = collect_unified_search_inputs(&conn, requested_project_dir.as_deref(), requested_source_kind.as_deref())?;
            let queued_before = docs.len() as u32;
            let mut indexed_documents = 0u32;
            for doc in docs.iter() {
                index_unified_search_document(&conn, doc)?;
                indexed_documents += 1;
            }
            Ok(UnifiedSearchIndexReport {
                mode: "unified-search-index".to_string(),
                requested_full: full,
                project_dir: requested_project_dir,
                source_kind: requested_source_kind,
                queued_before,
                indexed_documents,
                skipped_documents: 0,
                failed_documents: 0,
                removed_documents: 0,
                remaining_pending: 0,
            })
        }
    }


    #[cfg(feature = "semantic-memory")]
    pub fn unified_search_semantic(
        &self,
        project_dir: &str,
        requested_source_kinds: &[String],
        query: &str,
        limit: u32,
    ) -> Result<UnifiedSearchResponse> {
        let project_dir = normalize_project_dir(project_dir)?;
        let conn = self.conn.lock().unwrap();
        let query_embedding = build_text_embeddings(&[query.to_string()])?;
        let query_vector = query_embedding
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("semantic query embedding generation returned no vector"))?;
        let mut sql = String::from(
            "SELECT d.document_id, d.source_kind, d.source_id, d.project_dir, d.title, c.chunk_index, c.char_start, c.char_len, c.chunk_text, c.embedding_blob
             FROM unified_search_documents d
             JOIN unified_search_chunks c ON c.document_id = d.document_id
             WHERE d.project_dir = ?",
        );
        let mut params_vec: Vec<Box<dyn ToSql>> = vec![Box::new(project_dir.clone())];
        if !requested_source_kinds.is_empty() {
            sql.push_str(" AND d.source_kind IN (");
            for idx in 0..requested_source_kinds.len() {
                if idx > 0 { sql.push_str(", "); }
                sql.push('?');
                params_vec.push(Box::new(requested_source_kinds[idx].clone()));
            }
            sql.push(')');
        }
        let params_ref: Vec<&dyn ToSql> = params_vec.iter().map(|v| v.as_ref() as &dyn ToSql).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_ref.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)? as u32,
                row.get::<_, i64>(6)? as u32,
                row.get::<_, i64>(7)? as u32,
                row.get::<_, String>(8)?,
                row.get::<_, Vec<u8>>(9)?,
            ))
        })?;
        let mut grouped: BTreeMap<(String, String, String), Vec<(f64, UnifiedSearchSnippet, String)>> = BTreeMap::new();
        for row in rows {
            let (_document_id, source_kind, source_id, project_dir, title, chunk_index, char_start, char_len, chunk_text, blob) = row?;
            if let Some(embedding) = decode_embedding_blob(&blob)? {
                let score = cosine_similarity(&query_vector, &embedding) as f64;
                grouped.entry((source_kind, source_id, project_dir))
                    .or_default()
                    .push((score, UnifiedSearchSnippet { text: chunk_text, char_start, char_len }, format!("{}:{}", title, chunk_index)));
            }
        }
        let mut results = Vec::new();
        for ((source_kind, source_id, project_dir), mut chunks) in grouped {
            chunks.sort_by(|a,b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
            let best = chunks.first().cloned();
            if let Some((best_score, best_snippet, title)) = best {
                let mut score = best_score;
                if let Some((second_score, _, _)) = chunks.get(1) {
                    score += (0.10 * best_score).min(*second_score);
                }
                if let Some((third_score, _, _)) = chunks.get(2) {
                    score += (0.05 * best_score).min(*third_score);
                }
                let supporting_snippets = chunks.iter().skip(1).take(2)
                    .filter(|(candidate_score, _, _)| *candidate_score >= (0.50 * best_score))
                    .map(|(_, snippet, _)| snippet.clone())
                    .collect::<Vec<_>>();
                results.push(UnifiedSearchResult {
                    source_kind,
                    source_id,
                    project_dir,
                    score,
                    score_kind: UnifiedSearchMode::Semantic,
                    snippet: best_snippet.text.clone(),
                    primary_snippet: best_snippet.text,
                    supporting_snippets,
                    title,
                    node_type: None,
                    hashtags: Vec::new(),
                });
            }
        }
        results.sort_by(|a,b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
        results.truncate(limit.min(50) as usize);
        Ok(UnifiedSearchResponse {
            mode: UnifiedSearchMode::Semantic,
            degraded: false,
            degradation_reason: None,
            pending_index_count: 0,
            unavailable_source_kinds: Vec::new(),
            results,
        })
    }

    pub fn open_graph(&self, node_ids: Vec<String>, depth: u32, limit: u32) -> Result<MemoryGraph> {
        let depth = depth.min(MAX_GRAPH_DEPTH).max(1);
        let limit = clamp_limit(limit, DEFAULT_GRAPH_LIMIT, MAX_GRAPH_LIMIT);
        let conn = self.conn.lock().unwrap();
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut frontier: BTreeSet<String> = node_ids.into_iter().collect();
        let mut edges_by_id: BTreeMap<String, MemoryEdge> = BTreeMap::new();
        let mut truncated = false;

        for _ in 0..=depth {
            if visited.len() as u32 >= limit {
                truncated = true;
                break;
            }
            let current: Vec<String> = frontier.difference(&visited).cloned().collect();
            if current.is_empty() {
                break;
            }
            for node_id in current {
                if node_exists(&conn, &node_id)? {
                    visited.insert(node_id.clone());
                }
                if visited.len() as u32 >= limit {
                    truncated = true;
                    break;
                }
                let related = all_edges_for(&conn, &node_id)?;
                for edge in related {
                    frontier.insert(edge.from_node_id.clone());
                    frontier.insert(edge.to_node_id.clone());
                    edges_by_id.insert(edge.edge_id.clone(), edge);
                }
            }
        }

        let mut nodes = Vec::new();
        for node_id in &visited {
            if let Some(node) = get_node_locked(&conn, node_id)? {
                nodes.push(node);
            }
        }
        let mut edges: Vec<MemoryEdge> = edges_by_id
            .into_values()
            .filter(|edge| {
                visited.contains(&edge.from_node_id) && visited.contains(&edge.to_node_id)
            })
            .collect();
        edges.sort_by(|a, b| a.created_at_ms.cmp(&b.created_at_ms));
        Ok(MemoryGraph {
            nodes,
            edges,
            truncated,
            depth,
            limit,
        })
    }

    pub fn list_hashtags(
        &self,
        project_dir: &str,
        prefix: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MemoryHashtagStat>> {
        let limit = clamp_limit(limit, 50, 200);
        let normalized_prefix = prefix.and_then(|p| normalize_hashtag(p).ok());
        let project_dir = normalize_project_dir(project_dir)?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT h.hashtag, COUNT(*)
             FROM memory_node_hashtags h
             JOIN memory_nodes n ON n.node_id = h.node_id
             WHERE n.project_dir = ?1 AND (?2 IS NULL OR h.hashtag LIKE ?2 || '%')
             GROUP BY h.hashtag
             ORDER BY COUNT(*) DESC, h.hashtag ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![project_dir, normalized_prefix, limit as i64],
            |row| {
                Ok(MemoryHashtagStat {
                    hashtag: row.get(0)?,
                    node_count: row.get::<_, i64>(1)? as u32,
                })
            },
        )?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn get_node(&self, node_id: &str) -> Result<Option<MemoryNode>> {
        let conn = self.conn.lock().unwrap();
        get_node_locked(&conn, node_id)
    }

    fn get_edge(&self, edge_id: &str) -> Result<Option<MemoryEdge>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT edge_id, from_node_id, to_node_id, relation_type, metadata_json, created_at_ms
             FROM memory_edges WHERE edge_id = ?1",
            params![edge_id],
            map_edge_row,
        )
        .optional()
        .map_err(Into::into)
    }
}

fn normalize_optional_uuid(value: Option<String>) -> Result<String> {
    match value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(raw) => Ok(Uuid::parse_str(raw)?.to_string()),
        None => Ok(Uuid::new_v4().to_string()),
    }
}

pub fn normalize_project_dir(value: &str) -> Result<String> {
    let project_dir = value.trim();
    if project_dir.is_empty() {
        anyhow::bail!("project_dir must not be empty");
    }
    if project_dir == GLOBAL_PROJECT_DIR {
        return Ok(GLOBAL_PROJECT_DIR.to_string());
    }
    Ok(project_dir.to_string())
}

fn normalize_required_title(value: &str) -> Result<String> {
    let title = value.trim();
    if title.is_empty() {
        anyhow::bail!("title must not be empty");
    }
    Ok(title.to_string())
}

fn normalize_required_label(value: &str, field: &str) -> Result<String> {
    let label = value.trim().to_ascii_lowercase().replace('-', "_");
    if label.is_empty()
        || !label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        anyhow::bail!("{field} must contain only letters, digits, underscore, or hyphen");
    }
    Ok(label)
}

pub fn normalize_hashtag(value: &str) -> Result<String> {
    let mut tag = value.trim().to_ascii_lowercase();
    if tag.starts_with('#') {
        tag.remove(0);
    }
    tag = tag.replace('-', "_");
    if tag.is_empty()
        || !tag
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        anyhow::bail!("hashtags must contain only letters, digits, underscore, or hyphen");
    }
    Ok(format!("#{tag}"))
}

fn normalize_hashtags(values: Vec<String>) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    for value in values {
        seen.insert(normalize_hashtag(&value)?);
    }
    Ok(seen.into_iter().collect())
}

fn replace_hashtags(conn: &Connection, node_id: &str, hashtags: &[String]) -> Result<()> {
    conn.execute(
        "DELETE FROM memory_node_hashtags WHERE node_id = ?1",
        params![node_id],
    )?;
    for hashtag in hashtags {
        conn.execute(
            "INSERT OR IGNORE INTO memory_node_hashtags (node_id, hashtag) VALUES (?1, ?2)",
            params![node_id, hashtag],
        )?;
    }
    Ok(())
}

fn node_exists(conn: &Connection, node_id: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM memory_nodes WHERE node_id = ?1",
            params![node_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn get_node_locked(conn: &Connection, node_id: &str) -> Result<Option<MemoryNode>> {
    conn.query_row(
        "SELECT node_id, project_dir, node_type, title, content, metadata_json, created_at_ms, updated_at_ms
         FROM memory_nodes WHERE node_id = ?1",
        params![node_id],
        |row| map_node_row_with_conn(conn, row),
    )
    .optional()
    .map_err(Into::into)
}

fn map_node_row_with_conn(
    conn: &Connection,
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<MemoryNode> {
    let node_id: String = row.get(0)?;
    let hashtags = hashtags_for(conn, &node_id)?;
    Ok(MemoryNode {
        node_id,
        project_dir: row.get(1)?,
        node_type: row.get(2)?,
        title: row.get(3)?,
        content: row.get(4)?,
        metadata_json: row.get(5)?,
        created_at_ms: row.get(6)?,
        updated_at_ms: row.get(7)?,
        hashtags,
    })
}

fn hashtags_for(conn: &Connection, node_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT hashtag FROM memory_node_hashtags WHERE node_id = ?1 ORDER BY hashtag ASC",
    )?;
    let rows = stmt.query_map(params![node_id], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn map_edge_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEdge> {
    Ok(MemoryEdge {
        edge_id: row.get(0)?,
        from_node_id: row.get(1)?,
        to_node_id: row.get(2)?,
        relation_type: row.get(3)?,
        metadata_json: row.get(4)?,
        created_at_ms: row.get(5)?,
    })
}

fn edges_for(conn: &Connection, column: &str, node_id: &str) -> Result<Vec<MemoryEdge>> {
    let sql = format!(
        "SELECT edge_id, from_node_id, to_node_id, relation_type, metadata_json, created_at_ms
         FROM memory_edges WHERE {column} = ?1 ORDER BY created_at_ms ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![node_id], map_edge_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn all_edges_for(conn: &Connection, node_id: &str) -> Result<Vec<MemoryEdge>> {
    let mut stmt = conn.prepare(
        "SELECT edge_id, from_node_id, to_node_id, relation_type, metadata_json, created_at_ms
         FROM memory_edges
         WHERE from_node_id = ?1 OR to_node_id = ?1
         ORDER BY created_at_ms ASC",
    )?;
    let rows = stmt.query_map(params![node_id], map_edge_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn clamp_limit(value: u32, default: u32, max: u32) -> u32 {
    if value == 0 {
        default
    } else {
        value.min(max)
    }
}

pub fn parse_hashtags_value(value: Option<&Value>) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if let Some(array) = value.as_array() {
        return array
            .iter()
            .map(|item| {
                item.as_str()
                    .ok_or_else(|| anyhow::anyhow!("hashtags must be strings"))
                    .map(str::to_string)
            })
            .collect();
    }
    if let Some(text) = value.as_str() {
        return Ok(text
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|part| !part.trim().is_empty())
            .map(str::to_string)
            .collect());
    }
    anyhow::bail!("hashtags must be an array of strings or a string")
}

pub fn metadata_to_string(value: Option<&Value>) -> Result<Option<String>> {
    match value {
        Some(Value::Null) | None => Ok(None),
        Some(value) => Ok(Some(serde_json::to_string(value)?)),
    }
}

pub fn parse_nullable_string(value: Option<&Value>) -> Result<Option<Option<String>>> {
    match value {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => Ok(Some(Some(
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("value must be a string or null"))?
                .to_string(),
        ))),
    }
}

#[derive(Debug, Deserialize)]
pub struct OpenGraphArgs {
    pub node_ids: Option<Vec<String>>,
    pub node_id: Option<String>,
    pub depth: Option<u32>,
    pub limit: Option<u32>,
}

impl OpenGraphArgs {
    pub fn into_parts(self) -> (Vec<String>, u32, u32) {
        let mut node_ids = self.node_ids.unwrap_or_default();
        if let Some(node_id) = self.node_id {
            node_ids.push(node_id);
        }
        let depth = self.depth.unwrap_or(DEFAULT_GRAPH_DEPTH);
        let limit = self.limit.unwrap_or(DEFAULT_GRAPH_LIMIT);
        (node_ids, depth, limit)
    }
}

#[cfg(feature = "semantic-memory")]
fn register_memory_node_for_unified_search(
    conn: &Connection,
    node_id: &str,
    project_dir: &str,
    title: &str,
    content: Option<&str>,
    source_updated_at_ms: i64,
) -> Result<()> {
    let doc = UnifiedSearchDocumentInput {
        source_kind: "memory".to_string(),
        source_id: node_id.to_string(),
        project_dir: project_dir.to_string(),
        session_id: None,
        turn_seq: None,
        tool_call_id: None,
        title: title.to_string(),
        source_text: embedding_input_from_parts(title, content),
        source_updated_at_ms,
    };
    upsert_unified_search_document_pending(conn, &doc)
}

#[cfg(feature = "semantic-memory")]
fn embedding_input_from_parts(title: &str, content: Option<&str>) -> String {
    match content.map(str::trim).filter(|value| !value.is_empty()) {
        Some(content) => format!(
            "{}

{}",
            title, content
        ),
        None => title.to_string(),
    }
}

#[cfg(feature = "semantic-memory")]
fn encode_embedding_blob(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

#[cfg(feature = "semantic-memory")]
fn decode_embedding_blob(blob: &[u8]) -> Result<Option<Vec<f32>>> {
    if blob.is_empty() {
        return Ok(None);
    }
    if blob.len() % 4 != 0 {
        anyhow::bail!("embedding blob length {} is not divisible by 4", blob.len());
    }
    let mut values = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks_exact(4) {
        values.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(Some(values))
}

#[cfg(feature = "semantic-memory")]
fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (a, b) in left.iter().zip(right.iter()) {
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

#[cfg(feature = "semantic-memory")]
fn build_text_embeddings(texts: &[String]) -> Result<Vec<Vec<f32>>> {
    use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
    configure_fastembed_cache_dir()?;
    let mut model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::BGESmallZHV15))?;
    Ok(model.embed(texts.to_vec(), None)?)
}

#[cfg(feature = "semantic-memory")]
fn chunk_text_for_unified_search(source_text: &str) -> Vec<UnifiedSearchChunkDraft> {
    let chars: Vec<char> = source_text.chars().collect();
    if chars.is_empty() {
        return vec![UnifiedSearchChunkDraft {
            chunk_index: 0,
            char_start: 0,
            char_len: 0,
            chunk_text: String::new(),
        }];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let step = UNIFIED_SEARCH_CHUNK_LEN.saturating_sub(UNIFIED_SEARCH_CHUNK_OVERLAP).max(1);
    while start < chars.len() {
        let end = (start + UNIFIED_SEARCH_CHUNK_LEN).min(chars.len());
        let chunk_text: String = chars[start..end].iter().collect();
        chunks.push(UnifiedSearchChunkDraft {
            chunk_index: chunks.len() as u32,
            char_start: start as u32,
            char_len: (end - start) as u32,
            chunk_text,
        });
        if end == chars.len() {
            break;
        }
        start += step;
    }
    chunks
}

#[cfg(feature = "semantic-memory")]
fn tool_name_from_result_payload(raw: &str) -> Option<String> {
    serde_json::from_str::<Value>(raw).ok().and_then(|value| {
        value
            .get("tool_name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    })
}

#[cfg(feature = "semantic-memory")]
fn collect_unified_search_inputs(
    conn: &Connection,
    project_dir: Option<&str>,
    source_kind: Option<&str>,
) -> Result<Vec<UnifiedSearchDocumentInput>> {
    let mut docs = Vec::new();
    let mut memory_sql = String::from(
        "SELECT node_id, project_dir, node_type, title, content, updated_at_ms FROM memory_nodes WHERE 1=1",
    );
    let mut memory_params: Vec<Box<dyn ToSql>> = Vec::new();
    if let Some(project_dir) = project_dir {
        memory_sql.push_str(" AND project_dir = ?");
        memory_params.push(Box::new(project_dir.to_string()));
    }
    let memory_params_ref: Vec<&dyn ToSql> = memory_params.iter().map(|v| v.as_ref() as &dyn ToSql).collect();
    if source_kind.is_none() || source_kind == Some("memory") {
        let mut stmt = conn.prepare(&memory_sql)?;
        let rows = stmt.query_map(memory_params_ref.as_slice(), |row| {
            let title: String = row.get(3)?;
            let content: Option<String> = row.get(4)?;
            Ok(UnifiedSearchDocumentInput {
                source_kind: "memory".to_string(),
                source_id: row.get(0)?,
                project_dir: row.get(1)?,
                session_id: None,
                turn_seq: None,
                tool_call_id: None,
                title: title.clone(),
                source_text: match content.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                    Some(content) => format!("{}

{}", title, content),
                    None => title,
                },
                source_updated_at_ms: row.get(5)?,
            })
        })?;
        for row in rows {
            docs.push(row?);
        }
    }

    if source_kind.is_none() || matches!(source_kind, Some("chat_message") | Some("tool_call") | Some("tool_result")) {
        let mut sql = String::from(
            "SELECT m.message_id, m.session_id, t.turn_seq, m.role, m.content, m.tool_calls_json, m.tool_call_id, s.project_dir, t.created_at
             FROM agent_messages m
             JOIN agent_turns t ON m.turn_id = t.turn_id
             JOIN agent_sessions s ON m.session_id = s.session_id
             WHERE 1=1",
        );
        let mut params_vec: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(project_dir) = project_dir {
            sql.push_str(" AND s.project_dir = ?");
            params_vec.push(Box::new(project_dir.to_string()));
        }
        let params_ref: Vec<&dyn ToSql> = params_vec.iter().map(|v| v.as_ref() as &dyn ToSql).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_ref.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? as u32,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?,
            ))
        })?;
        for row in rows {
            let (message_id, session_id, turn_seq, role, content, tool_calls_json, tool_call_id, project_dir, created_at) = row?;
            if source_kind.is_none() || source_kind == Some("chat_message") {
                if let Some(doc) = build_chat_message_document_input(
                    message_id,
                    &session_id,
                    turn_seq,
                    &role,
                    content.as_deref(),
                    tool_calls_json.as_deref(),
                    &project_dir,
                    created_at,
                ) {
                    docs.push(doc);
                }
            }
            if (source_kind.is_none() || source_kind == Some("tool_call")) && tool_calls_json.as_deref().map(|v| !v.trim().is_empty()).unwrap_or(false) {
                let tool_json = tool_calls_json.clone().unwrap_or_default();
                if let Ok(Value::Array(calls)) = serde_json::from_str::<Value>(&tool_json) {
                    for call in calls {
                        let tool_name = call.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or("tool_call");
                        let tool_args = call.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()).unwrap_or("");
                        docs.push(UnifiedSearchDocumentInput {
                            source_kind: "tool_call".to_string(),
                            source_id: format!("{}:{}:{}", message_id, tool_name, docs.len()),
                            project_dir: project_dir.clone(),
                            session_id: Some(session_id.clone()),
                            turn_seq: Some(turn_seq),
                            tool_call_id: call.get("id").and_then(|v| v.as_str()).map(str::to_string),
                            title: tool_name.to_string(),
                            source_text: format!("{}

{}", tool_name, tool_args),
                            source_updated_at_ms: created_at * 1000,
                        });
                    }
                }
            }
            if (source_kind.is_none() || source_kind == Some("tool_result")) && role == "tool" {
                let text = content.clone().unwrap_or_default();
                if !text.trim().is_empty() {
                    let title = tool_name_from_result_payload(&text).unwrap_or_else(|| "tool_result".to_string());
                    docs.push(UnifiedSearchDocumentInput {
                        source_kind: "tool_result".to_string(),
                        source_id: message_id.to_string(),
                        project_dir: project_dir.clone(),
                        session_id: Some(session_id.clone()),
                        turn_seq: Some(turn_seq),
                        tool_call_id: tool_call_id.clone(),
                        title,
                        source_text: text,
                        source_updated_at_ms: created_at * 1000,
                    });
                }
            }
        }
    }
    docs.sort_by(|a, b| b.source_updated_at_ms.cmp(&a.source_updated_at_ms));
    Ok(docs)
}

#[cfg(feature = "semantic-memory")]
fn clear_unified_search_scope(
    conn: &Connection,
    project_dir: Option<&str>,
    source_kind: Option<&str>,
) -> Result<()> {
    match (project_dir, source_kind) {
        (Some(project_dir), Some(source_kind)) => {
            conn.execute(
                "DELETE FROM unified_search_documents WHERE project_dir = ?1 AND source_kind = ?2",
                params![project_dir, source_kind],
            )?;
        }
        (Some(project_dir), None) => {
            conn.execute(
                "DELETE FROM unified_search_documents WHERE project_dir = ?1",
                params![project_dir],
            )?;
        }
        (None, Some(source_kind)) => {
            conn.execute(
                "DELETE FROM unified_search_documents WHERE source_kind = ?1",
                params![source_kind],
            )?;
        }
        (None, None) => {
            conn.execute("DELETE FROM unified_search_documents", [])?;
        }
    }
    Ok(())
}

#[cfg(feature = "semantic-memory")]
#[cfg(feature = "semantic-memory")]
fn build_chat_message_document_input(
    message_id: i64,
    session_id: &str,
    turn_seq: u32,
    role: &str,
    content: Option<&str>,
    tool_calls_json: Option<&str>,
    project_dir: &str,
    created_at_s: i64,
) -> Option<UnifiedSearchDocumentInput> {
    if role == "tool" || tool_calls_json.is_some() {
        return None;
    }
    let text = content.unwrap_or_default().trim();
    if text.is_empty() {
        return None;
    }
    Some(UnifiedSearchDocumentInput {
        source_kind: "chat_message".to_string(),
        source_id: message_id.to_string(),
        project_dir: project_dir.to_string(),
        session_id: Some(session_id.to_string()),
        turn_seq: Some(turn_seq),
        tool_call_id: None,
        title: role.to_string(),
        source_text: text.to_string(),
        source_updated_at_ms: created_at_s * 1000,
    })
}

#[cfg(feature = "semantic-memory")]
fn upsert_unified_search_document_pending(conn: &Connection, doc: &UnifiedSearchDocumentInput) -> Result<()> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT document_id FROM unified_search_documents WHERE source_kind = ?1 AND source_id = ?2 AND project_dir = ?3",
            params![doc.source_kind, doc.source_id, doc.project_dir],
            |row| row.get(0),
        )
        .optional()?;
    let document_id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM unified_search_chunks WHERE document_id = ?1",
        params![document_id],
    )?;
    tx.execute(
        "INSERT INTO unified_search_documents (document_id, source_kind, source_id, project_dir, session_id, turn_seq, tool_call_id, title, source_text, source_updated_at_ms, chunking_version, embedding_model, embedding_state, last_indexed_at_ms, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'pending', NULL, NULL)
         ON CONFLICT(document_id) DO UPDATE SET
             session_id=excluded.session_id,
             turn_seq=excluded.turn_seq,
             tool_call_id=excluded.tool_call_id,
             title=excluded.title,
             source_text=excluded.source_text,
             source_updated_at_ms=excluded.source_updated_at_ms,
             chunking_version=excluded.chunking_version,
             embedding_model=excluded.embedding_model,
             embedding_state='pending',
             last_indexed_at_ms=NULL,
             last_error=NULL",
        params![
            document_id,
            doc.source_kind,
            doc.source_id,
            doc.project_dir,
            doc.session_id,
            doc.turn_seq.map(|v| v as i64),
            doc.tool_call_id,
            doc.title,
            doc.source_text,
            doc.source_updated_at_ms,
            UNIFIED_SEARCH_CHUNKING_VERSION,
            DEFAULT_SEMANTIC_MODEL,
        ],
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(feature = "semantic-memory")]
fn pending_document_to_input(pending: &PendingUnifiedSearchDocument) -> UnifiedSearchDocumentInput {
    UnifiedSearchDocumentInput {
        source_kind: pending.source_kind.clone(),
        source_id: pending.source_id.clone(),
        project_dir: pending.project_dir.clone(),
        session_id: pending.session_id.clone(),
        turn_seq: pending.turn_seq,
        tool_call_id: pending.tool_call_id.clone(),
        title: pending.title.clone(),
        source_text: pending.source_text.clone(),
        source_updated_at_ms: pending.source_updated_at_ms,
    }
}

#[cfg(feature = "semantic-memory")]
fn list_pending_unified_search_documents(
    conn: &Connection,
    project_dir: &str,
    source_kind: &str,
    limit: u32,
) -> Result<Vec<PendingUnifiedSearchDocument>> {
    let mut stmt = conn.prepare(
        "SELECT document_id, source_kind, source_id, project_dir, session_id, turn_seq, tool_call_id, title, source_text, source_updated_at_ms
         FROM unified_search_documents
         WHERE project_dir = ?1 AND source_kind = ?2 AND embedding_state = 'pending'
         ORDER BY (last_error IS NOT NULL) ASC, source_updated_at_ms DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![project_dir, source_kind, limit as i64], |row| {
        Ok(PendingUnifiedSearchDocument {
            source_kind: row.get(1)?,
            source_id: row.get(2)?,
            project_dir: row.get(3)?,
            session_id: row.get(4)?,
            turn_seq: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
            tool_call_id: row.get(6)?,
            title: row.get(7)?,
            source_text: row.get(8)?,
            source_updated_at_ms: row.get(9)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

#[cfg(feature = "semantic-memory")]
fn mark_unified_search_document_failed(
    conn: &Connection,
    source_kind: &str,
    source_id: &str,
    project_dir: &str,
    error: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE unified_search_documents
         SET embedding_state = 'failed',
             last_error = ?4
         WHERE source_kind = ?1 AND source_id = ?2 AND project_dir = ?3",
        params![source_kind, source_id, project_dir, error],
    )?;
    Ok(())
}

#[cfg(feature = "semantic-memory")]
fn index_pending_unified_search_document(conn: &Connection, pending: &PendingUnifiedSearchDocument) -> Result<()> {
    let doc = pending_document_to_input(pending);
    match index_unified_search_document(conn, &doc) {
        Ok(()) => Ok(()),
        Err(error) => {
            let error_text = error.to_string();
            let _ = mark_unified_search_document_failed(
                conn,
                &pending.source_kind,
                &pending.source_id,
                &pending.project_dir,
                &error_text,
            );
            Err(error)
        }
    }
}

#[cfg(feature = "semantic-memory")]
fn index_unified_search_document(conn: &Connection, doc: &UnifiedSearchDocumentInput) -> Result<()> {
    let now_ms = now_unix_ms();
    let existing: Option<String> = conn
        .query_row(
            "SELECT document_id FROM unified_search_documents WHERE source_kind = ?1 AND source_id = ?2 AND project_dir = ?3",
            params![doc.source_kind, doc.source_id, doc.project_dir],
            |row| row.get(0),
        )
        .optional()?;
    let document_id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
    let chunks = chunk_text_for_unified_search(&doc.source_text);
    let embeddings = build_text_embeddings(&chunks.iter().map(|chunk| chunk.chunk_text.clone()).collect::<Vec<_>>())?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM unified_search_chunks WHERE document_id = ?1",
        params![document_id],
    )?;
    tx.execute(
        "INSERT INTO unified_search_documents (document_id, source_kind, source_id, project_dir, session_id, turn_seq, tool_call_id, title, source_text, source_updated_at_ms, chunking_version, embedding_model, embedding_state, last_indexed_at_ms, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'ready', ?13, NULL)
         ON CONFLICT(document_id) DO UPDATE SET
             session_id=excluded.session_id,
             turn_seq=excluded.turn_seq,
             tool_call_id=excluded.tool_call_id,
             title=excluded.title,
             source_text=excluded.source_text,
             source_updated_at_ms=excluded.source_updated_at_ms,
             chunking_version=excluded.chunking_version,
             embedding_model=excluded.embedding_model,
             embedding_state='ready',
             last_indexed_at_ms=excluded.last_indexed_at_ms,
             last_error=NULL",
        params![
            document_id, doc.source_kind, doc.source_id, doc.project_dir, doc.session_id, doc.turn_seq.map(|v| v as i64), doc.tool_call_id, doc.title, doc.source_text, doc.source_updated_at_ms,
            UNIFIED_SEARCH_CHUNKING_VERSION, DEFAULT_SEMANTIC_MODEL, now_ms
        ],
    )?;
    for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
        tx.execute(
            "INSERT INTO unified_search_chunks (chunk_id, document_id, chunk_index, char_start, char_len, chunk_text, token_start, token_len, embedding_model, embedding_dim, embedding_blob, source_updated_at_ms, indexed_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, ?7, ?8, ?9, ?10, ?11)",
            params![
                Uuid::new_v4().to_string(), document_id, chunk.chunk_index as i64, chunk.char_start as i64, chunk.char_len as i64, chunk.chunk_text, DEFAULT_SEMANTIC_MODEL, embedding.len() as i64, encode_embedding_blob(embedding), doc.source_updated_at_ms, now_ms
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

