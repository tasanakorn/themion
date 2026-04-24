use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, ToSql};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
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

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
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

pub fn init_schema(conn: &Connection, fts5: bool) -> Result<()> {
    conn.execute_batch(MEMORY_SCHEMA_BASE)?;
    migrate_project_dir(conn)?;
    if fts5 {
        conn.execute_batch(MEMORY_SCHEMA_FTS5)?;
    }
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

    pub fn search_nodes(&self, mut args: SearchNodesArgs) -> Result<Vec<MemoryNode>> {
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
        Ok(out)
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
