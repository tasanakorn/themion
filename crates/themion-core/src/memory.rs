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

#[cfg(feature = "semantic-memory")]
const DEFAULT_SEMANTIC_MODEL: &str = "bge-micro-v2";
#[cfg(feature = "semantic-memory")]
const DEFAULT_SEMANTIC_CANDIDATE_LIMIT: usize = 200;
#[cfg(feature = "semantic-memory")]
const DEFAULT_SEMANTIC_FULL_SCAN_LIMIT: usize = 10_000;
#[cfg(feature = "semantic-memory")]
const DEFAULT_SEMANTIC_INDEX_BATCH_SIZE: usize = 64;
#[cfg(feature = "semantic-memory")]
const MEMORY_SCHEMA_SEMANTIC: &str = "
CREATE TABLE IF NOT EXISTS memory_node_embeddings (
    node_id TEXT PRIMARY KEY REFERENCES memory_nodes(node_id) ON DELETE CASCADE,
    embedding_model TEXT NOT NULL,
    embedding_dim INTEGER NOT NULL,
    embedding_blob BLOB NOT NULL,
    source_updated_at_ms INTEGER NOT NULL,
    indexed_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_node_embeddings_model_updated
    ON memory_node_embeddings(embedding_model, source_updated_at_ms DESC);

";

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

#[cfg(feature = "semantic-memory")]
fn init_semantic_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(MEMORY_SCHEMA_SEMANTIC)?;
    Ok(())
}

pub fn init_schema(conn: &Connection, fts5: bool) -> Result<()> {
    conn.execute_batch(MEMORY_SCHEMA_BASE)?;
    migrate_project_dir(conn)?;
    if fts5 {
        conn.execute_batch(MEMORY_SCHEMA_FTS5)?;
    }
    #[cfg(feature = "semantic-memory")]
    init_semantic_schema(conn)?;
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
    pub mode: MemorySearchMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemorySearchResponse {
    pub mode: MemorySearchMode,
    pub degraded: bool,
    pub degradation_reason: Option<String>,
    pub pending_index_count: u32,
    pub nodes: Vec<MemoryNode>,
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
pub enum MemorySearchMode {
    Fts,
    Semantic,
}

impl MemorySearchMode {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "fts" => Some(Self::Fts),
            "semantic" => Some(Self::Semantic),
            _ => None,
        }
    }
}

impl Default for MemorySearchMode {
    fn default() -> Self {
        Self::Fts
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
        write_node_embedding_now(&conn, &node_id, &title, args.content.as_deref(), now_ms)?;
        drop(conn);
        self.get_node(&node_id)?
            .ok_or_else(|| anyhow::anyhow!("memory node insert failed"))
    }

    pub fn update_node(&self, node_id: &str, args: UpdateNodeArgs) -> Result<Option<MemoryNode>> {
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        let existing: Option<(String, String, Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT node_type, title, content, metadata_json FROM memory_nodes WHERE node_id = ?1",
                params![node_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((old_type, old_title, old_content, old_metadata)) = existing else {
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
        write_node_embedding_now(&conn, node_id, &title, content.as_deref(), now_ms)?;
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
            MemorySearchMode::Fts => self.search_nodes_fts(args),
            MemorySearchMode::Semantic => {
                #[cfg(feature = "semantic-memory")]
                {
                    self.search_nodes_semantic(args)
                }
                #[cfg(not(feature = "semantic-memory"))]
                {
                    let _ = args;
                    Ok(MemorySearchResponse {
                        mode: MemorySearchMode::Semantic,
                        degraded: true,
                        degradation_reason: Some("semantic retrieval unavailable: themion was built without the semantic-memory feature".to_string()),
                        pending_index_count: 0,
                        nodes: Vec::new(),
                    })
                }
            }
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
            mode: MemorySearchMode::Fts,
            degraded: false,
            degradation_reason: None,
            pending_index_count: 0,
            nodes: out,
        })
    }

    #[cfg(feature = "semantic-memory")]
    pub fn index_pending_embeddings(&self, force_full: bool) -> Result<MemoryIndexReport> {
        let model_tag = DEFAULT_SEMANTIC_MODEL.to_string();
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();

        let queued_before = 0;
        let candidates = pending_embedding_candidates(&conn, force_full)?;
        let scanned_candidates = candidates.len() as u32;

        let found = candidates
            .iter()
            .map(|candidate| MemoryIndexNodeReport {
                node_id: candidate.node_id.clone(),
                title: candidate.title.clone(),
                reason: candidate.status.reason().to_string(),
            })
            .collect::<Vec<_>>();

        let mut to_regenerate = Vec::new();
        let mut skipped = Vec::new();
        for candidate in candidates {
            if candidate.status.needs_regeneration() {
                to_regenerate.push(candidate);
            } else {
                skipped.push(MemoryIndexNodeReport {
                    node_id: candidate.node_id.clone(),
                    title: candidate.title.clone(),
                    reason: candidate.status.reason().to_string(),
                });
            }
        }

        let mut indexed = Vec::new();
        let failures = Vec::new();
        let mut indexed_nodes = 0u32;

        if !to_regenerate.is_empty() {
            for chunk in to_regenerate.chunks(DEFAULT_SEMANTIC_INDEX_BATCH_SIZE) {
                let texts = chunk
                    .iter()
                    .map(PendingEmbeddingCandidate::embedding_input)
                    .collect::<Vec<_>>();
                let embeddings = build_text_embeddings(&texts)?;
                if embeddings.len() != chunk.len() {
                    anyhow::bail!(
                        "semantic embedding count mismatch: got {} vectors for {} nodes",
                        embeddings.len(),
                        chunk.len()
                    );
                }

                let tx = conn.unchecked_transaction()?;
                for (candidate, embedding) in chunk.iter().zip(embeddings.iter()) {
                    write_embedding_row(&tx, candidate, embedding, &model_tag, now_ms)?;
                    indexed.push(MemoryIndexNodeReport {
                        node_id: candidate.node_id.clone(),
                        title: candidate.title.clone(),
                        reason: candidate.status.reason().to_string(),
                    });
                    indexed_nodes += 1;
                }
                tx.commit()?;
            }
            if force_full {
                let tx = conn.unchecked_transaction()?;
                remove_stale_embeddings_without_nodes(&tx)?;
                tx.commit()?;
            }
        } else if force_full {
            let tx = conn.unchecked_transaction()?;
            remove_stale_embeddings_without_nodes(&tx)?;
            tx.commit()?;
        }

        let removed_stale_embeddings = 0;
        let remaining_pending = 0;

        Ok(MemoryIndexReport {
            mode: "semantic-memory-index".to_string(),
            requested_full: force_full,
            queued_before,
            scanned_candidates,
            indexed_nodes,
            skipped_nodes: skipped.len() as u32,
            removed_stale_embeddings,
            remaining_pending,
            failed_nodes: failures.len() as u32,
            found,
            indexed,
            skipped,
            failures,
            model_tag,
        })
    }

    #[cfg(feature = "semantic-memory")]
    fn search_nodes_semantic(&self, args: SearchNodesArgs) -> Result<MemorySearchResponse> {
        let query = args
            .query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("semantic search requires a non-empty query"))?
            .to_string();
        let conn = self.conn.lock().unwrap();
        let pending_index_count = 0;
        let candidate_limit = args.limit.max(1).min(MAX_SEARCH_LIMIT) as usize;
        let candidates = semantic_candidates_for_query(
            &conn,
            &args,
            DEFAULT_SEMANTIC_CANDIDATE_LIMIT.max(candidate_limit),
        )?;
        if candidates.is_empty() {
            return Ok(MemorySearchResponse {
                mode: MemorySearchMode::Semantic,
                degraded: false,
                degradation_reason: None,
                pending_index_count,
                nodes: Vec::new(),
            });
        }
        let query_embedding = build_text_embeddings(&[query])?;
        let query_vector = query_embedding.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!("semantic query embedding generation returned no vector")
        })?;
        let mut scored = Vec::new();
        for (node, blob) in candidates {
            if let Some(embedding) = decode_embedding_blob(&blob)? {
                let score = cosine_similarity(&query_vector, &embedding);
                scored.push((score, node));
            }
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        let nodes = scored
            .into_iter()
            .take(candidate_limit)
            .map(|(_, node)| node)
            .collect();
        Ok(MemorySearchResponse {
            mode: MemorySearchMode::Semantic,
            degraded: false,
            degradation_reason: None,
            pending_index_count,
            nodes,
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
#[derive(Debug, Clone)]
struct PendingEmbeddingCandidate {
    node_id: String,
    title: String,
    content: Option<String>,
    updated_at_ms: i64,
    status: PendingEmbeddingStatus,
}

#[cfg(feature = "semantic-memory")]
impl PendingEmbeddingCandidate {
    fn embedding_input(&self) -> String {
        embedding_input_from_parts(&self.title, self.content.as_deref())
    }
}

#[cfg(feature = "semantic-memory")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingEmbeddingStatus {
    Missing,
    Stale,
    Current,
}

#[cfg(feature = "semantic-memory")]
impl PendingEmbeddingStatus {
    fn needs_regeneration(self) -> bool {
        matches!(self, Self::Missing | Self::Stale)
    }

    fn reason(self) -> &'static str {
        match self {
            Self::Missing => "missing_embedding",
            Self::Stale => "stale_embedding",
            Self::Current => "current",
        }
    }
}

#[cfg(feature = "semantic-memory")]
fn write_node_embedding_now(
    conn: &Connection,
    node_id: &str,
    title: &str,
    content: Option<&str>,
    source_updated_at_ms: i64,
) -> Result<()> {
    let text = embedding_input_from_parts(title, content);
    let embeddings = build_text_embeddings(&[text])?;
    let embedding = embeddings
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("semantic embedding generation returned no vector"))?;
    write_embedding_values(
        conn,
        node_id,
        &embedding,
        source_updated_at_ms,
        now_unix_ms(),
    )
}

#[cfg(feature = "semantic-memory")]
fn pending_embedding_candidates(
    conn: &Connection,
    force_full: bool,
) -> Result<Vec<PendingEmbeddingCandidate>> {
    let scan_limit = if force_full {
        DEFAULT_SEMANTIC_FULL_SCAN_LIMIT
    } else {
        DEFAULT_SEMANTIC_CANDIDATE_LIMIT
    } as i64;
    let mut stmt = conn.prepare(
        "SELECT n.node_id, n.title, n.content, n.updated_at_ms,
                e.embedding_model, e.source_updated_at_ms
         FROM memory_nodes n
         LEFT JOIN memory_node_embeddings e ON e.node_id = n.node_id
         ORDER BY n.updated_at_ms DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![scan_limit], |row| {
        let existing_model: Option<String> = row.get(4)?;
        let existing_source_updated_at_ms: Option<i64> = row.get(5)?;
        let updated_at_ms: i64 = row.get(3)?;
        let status = if existing_model.is_none() || existing_source_updated_at_ms.is_none() {
            PendingEmbeddingStatus::Missing
        } else if existing_model.as_deref() != Some(DEFAULT_SEMANTIC_MODEL)
            || existing_source_updated_at_ms.unwrap_or_default() < updated_at_ms
        {
            PendingEmbeddingStatus::Stale
        } else {
            PendingEmbeddingStatus::Current
        };
        Ok(PendingEmbeddingCandidate {
            node_id: row.get(0)?,
            title: row.get(1)?,
            content: row.get(2)?,
            updated_at_ms,
            status,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        let candidate = row?;
        if force_full || candidate.status != PendingEmbeddingStatus::Current {
            out.push(candidate);
        }
    }
    Ok(out)
}

#[cfg(feature = "semantic-memory")]
fn write_embedding_row(
    conn: &Connection,
    candidate: &PendingEmbeddingCandidate,
    embedding: &[f32],
    model_tag: &str,
    indexed_at_ms: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_node_embeddings (node_id, embedding_model, embedding_dim, embedding_blob, source_updated_at_ms, indexed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(node_id) DO UPDATE SET
           embedding_model = excluded.embedding_model,
           embedding_dim = excluded.embedding_dim,
           embedding_blob = excluded.embedding_blob,
           source_updated_at_ms = excluded.source_updated_at_ms,
           indexed_at_ms = excluded.indexed_at_ms",
        params![
            candidate.node_id,
            model_tag,
            embedding.len() as i64,
            encode_embedding_blob(embedding),
            candidate.updated_at_ms,
            indexed_at_ms,
        ],
    )?;
    Ok(())
}

#[cfg(feature = "semantic-memory")]
fn write_embedding_values(
    conn: &Connection,
    node_id: &str,
    embedding: &[f32],
    source_updated_at_ms: i64,
    indexed_at_ms: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_node_embeddings (node_id, embedding_model, embedding_dim, embedding_blob, source_updated_at_ms, indexed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(node_id) DO UPDATE SET
           embedding_model = excluded.embedding_model,
           embedding_dim = excluded.embedding_dim,
           embedding_blob = excluded.embedding_blob,
           source_updated_at_ms = excluded.source_updated_at_ms,
           indexed_at_ms = excluded.indexed_at_ms",
        params![
            node_id,
            DEFAULT_SEMANTIC_MODEL,
            embedding.len() as i64,
            encode_embedding_blob(embedding),
            source_updated_at_ms,
            indexed_at_ms,
        ],
    )?;
    Ok(())
}

#[cfg(feature = "semantic-memory")]
fn remove_stale_embeddings_without_nodes(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM memory_node_embeddings WHERE node_id NOT IN (SELECT node_id FROM memory_nodes)",
        [],
    )?;
    Ok(())
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
fn semantic_candidates_for_query(
    conn: &Connection,
    args: &SearchNodesArgs,
    limit: usize,
) -> Result<Vec<(MemoryNode, Vec<u8>)>> {
    let mut sql = "SELECT n.node_id, n.project_dir, n.node_type, n.title, n.content, n.metadata_json, n.created_at_ms, n.updated_at_ms, e.embedding_blob
         FROM memory_nodes n
         JOIN memory_node_embeddings e ON e.node_id = n.node_id
         WHERE e.embedding_model = ?"
        .to_string();
    let mut params_vec: Vec<Box<dyn ToSql>> = vec![Box::new(DEFAULT_SEMANTIC_MODEL.to_string())];
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
            " AND EXISTS (SELECT 1 FROM memory_edges e2
                   WHERE (e2.from_node_id = n.node_id OR e2.to_node_id = n.node_id)
                     AND e2.relation_type = ?",
        );
        params_vec.push(Box::new(relation_type));
        if let Some(linked) = args
            .linked_node_id
            .as_ref()
            .filter(|v| !v.trim().is_empty())
        {
            sql.push_str(" AND (e2.from_node_id = ? OR e2.to_node_id = ?)");
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
            " AND EXISTS (SELECT 1 FROM memory_edges e2
                   WHERE (e2.from_node_id = n.node_id AND e2.to_node_id = ?)
                      OR (e2.to_node_id = n.node_id AND e2.from_node_id = ?))",
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
    params_vec.push(Box::new(limit as i64));
    let params_ref: Vec<&dyn ToSql> = params_vec
        .iter()
        .map(|v| v.as_ref() as &dyn ToSql)
        .collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref.as_slice(), |row| {
        Ok((
            map_node_row_with_conn(conn, row)?,
            row.get::<_, Vec<u8>>(8)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
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
