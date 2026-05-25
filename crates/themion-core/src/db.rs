use crate::memory::AppendedChatMessageIndexArgs;
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Row};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const BLOCKED_RETRY_COOLDOWN_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TurnMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version_dirty: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl TurnMeta {
    pub fn to_json(&self) -> Result<Option<String>> {
        if self.app_version.is_none()
            && self.app_version_hash.is_none()
            && self.app_version_dirty.is_none()
            && self.profile.is_none()
            && self.provider.is_none()
            && self.model.is_none()
        {
            return Ok(None);
        }
        serde_json::to_string(self)
            .context("failed to serialize turn meta")
            .map(Some)
    }

    pub fn from_json(raw: Option<&str>) -> Option<Self> {
        let raw = raw?.trim();
        if raw.is_empty() {
            return None;
        }
        serde_json::from_str(raw).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRecord {
    pub turn_id: i64,
    pub session_id: String,
    pub turn_seq: u32,
    pub meta_json: Option<String>,
    pub meta: Option<TurnMeta>,
}

fn map_turn_row(row: &Row<'_>) -> rusqlite::Result<TurnRecord> {
    let meta_json: Option<String> = row.get(3)?;
    let meta = TurnMeta::from_json(meta_json.as_deref());
    Ok(TurnRecord {
        turn_id: row.get(0)?,
        session_id: row.get(1)?,
        turn_seq: row.get::<_, i64>(2)? as u32,
        meta_json,
        meta,
    })
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub struct DbHandle {
    conn: Mutex<Connection>,
    fts5: bool,
}

const SCHEMA_BASE: &str = "
CREATE TABLE IF NOT EXISTS agent_sessions (
    session_id     TEXT PRIMARY KEY,
    project_dir    TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    is_interactive INTEGER NOT NULL DEFAULT 0,
    current_workflow TEXT,
    current_phase TEXT,
    workflow_status TEXT,
    current_phase_result TEXT,
    current_agent TEXT,
    workflow_last_updated_turn_seq INTEGER,
    workflow_started_at INTEGER,
    workflow_updated_at INTEGER,
    workflow_completed_at INTEGER,
    current_phase_retry_count INTEGER,
    current_phase_retry_limit INTEGER,
    previous_phase_retry_count INTEGER,
    previous_phase_retry_limit INTEGER,
    phase_entered_via TEXT
);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_project ON agent_sessions(project_dir, created_at);

CREATE TABLE IF NOT EXISTS agent_turns (
    turn_id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id       TEXT NOT NULL REFERENCES agent_sessions(session_id),
    turn_seq         INTEGER NOT NULL,
    tokens_in        INTEGER NOT NULL DEFAULT 0,
    tokens_out       INTEGER NOT NULL DEFAULT 0,
    tokens_cached    INTEGER NOT NULL DEFAULT 0,
    llm_rounds       INTEGER NOT NULL DEFAULT 0,
    tool_calls_count INTEGER NOT NULL DEFAULT 0,
    created_at       INTEGER NOT NULL,
    workflow_name TEXT,
    phase_start TEXT,
    phase_end TEXT,
    workflow_status_at_start TEXT,
    phase_result_at_start TEXT,
    workflow_status_at_end TEXT,
    phase_result_at_end TEXT,
    workflow_continues_after_turn INTEGER,
    turn_end_reason TEXT,
    meta TEXT
);

CREATE TABLE IF NOT EXISTS agent_messages (
    message_id      INTEGER PRIMARY KEY AUTOINCREMENT,
    turn_id         INTEGER NOT NULL REFERENCES agent_turns(turn_id),
    session_id      TEXT NOT NULL REFERENCES agent_sessions(session_id),
    seq             INTEGER NOT NULL,
    role            TEXT NOT NULL,
    content         TEXT,
    tool_calls_json TEXT,
    tool_call_id    TEXT,
    workflow_name   TEXT,
    phase_name      TEXT
);
CREATE INDEX IF NOT EXISTS idx_agent_messages_session_seq ON agent_messages(session_id, message_id);

CREATE TABLE IF NOT EXISTS agent_workflow_transitions (
    transition_id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES agent_sessions(session_id),
    turn_id INTEGER REFERENCES agent_turns(turn_id),
    turn_seq INTEGER,
    workflow_name TEXT NOT NULL,
    from_phase TEXT,
    to_phase TEXT NOT NULL,
    workflow_status TEXT NOT NULL,
    transition_kind TEXT NOT NULL,
    trigger_source TEXT,
    message_id INTEGER,
    retry_current_count INTEGER,
    retry_previous_count INTEGER,
    phase_entered_via TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_workflow_transitions_session_time
    ON agent_workflow_transitions(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_agent_workflow_transitions_turn
    ON agent_workflow_transitions(turn_id);

CREATE TABLE IF NOT EXISTS board_notes (
    note_id TEXT PRIMARY KEY,
    note_kind TEXT NOT NULL DEFAULT 'work_request',
    origin_note_id TEXT,
    completion_notified_at_ms INTEGER,
    from_instance TEXT,
    from_agent_id TEXT,
    to_instance TEXT NOT NULL,
    to_agent_id TEXT NOT NULL,
    body TEXT NOT NULL,
    column_name TEXT NOT NULL,
    result_text TEXT,
    injection_state TEXT NOT NULL,
    blocked_until_ms INTEGER,
    claimed_by_instance TEXT,
    claimed_by_agent_id TEXT,
    claimed_at_ms INTEGER,
    meta_json TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    injected_at_ms INTEGER
);
CREATE INDEX IF NOT EXISTS idx_board_notes_target_column_created
    ON board_notes(to_instance, to_agent_id, column_name, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_board_notes_target_injection
    ON board_notes(to_instance, to_agent_id, injection_state, created_at_ms);
CREATE INDEX IF NOT EXISTS idx_board_notes_target_blocked_until
    ON board_notes(to_instance, to_agent_id, column_name, blocked_until_ms, created_at_ms);

CREATE TABLE IF NOT EXISTS scheduled_tasks (
    schedule_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    schedule_kind TEXT NOT NULL,
    due_at_ms INTEGER,
    cron_expr TEXT,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    next_due_at_ms INTEGER,
    target_agent_id TEXT,
    owner_instance TEXT,
    busy_policy TEXT NOT NULL,
    retry_after_ms INTEGER,
    last_due_at_ms INTEGER,
    last_run_at_ms INTEGER,
    last_status TEXT NOT NULL,
    last_status_detail TEXT,
    last_handling_agent_id TEXT,
    claimed_by_instance TEXT,
    claimed_by_agent_id TEXT,
    claimed_due_at_ms INTEGER,
    claimed_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_due
    ON scheduled_tasks(enabled, next_due_at_ms, retry_after_ms, schedule_id);
CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_target
    ON scheduled_tasks(target_agent_id, enabled, next_due_at_ms);
";

const SCHEMA_FTS5: &str = "
CREATE VIRTUAL TABLE IF NOT EXISTS agent_messages_fts USING fts5(
    content,
    content='agent_messages',
    content_rowid='message_id',
    tokenize='porter unicode61'
);
CREATE TRIGGER IF NOT EXISTS agent_messages_ai AFTER INSERT ON agent_messages BEGIN
    INSERT INTO agent_messages_fts(rowid, content) VALUES (new.message_id, new.content);
END;
";

fn has_fts5(conn: &Connection) -> bool {
    let mut stmt = match conn.prepare("PRAGMA compile_options") {
        Ok(s) => s,
        Err(_) => return false,
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let has = rows.filter_map(|r| r.ok()).any(|o| o == "ENABLE_FTS5");
    has
}

fn ensure_column(conn: &Connection, table: &str, col: &str, decl: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let exists = cols.filter_map(|r| r.ok()).any(|name| name == col);
    if !exists {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl}"), [])?;
    }
    Ok(())
}

fn generate_note_slug(note_id: &str, body: &str) -> String {
    let mut slug = body
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-');
    let prefix: String = if slug.is_empty() {
        "note".to_string()
    } else {
        slug.chars().take(32).collect()
    };
    let suffix: String = note_id.chars().filter(|c| *c != '-').take(8).collect();
    format!("{prefix}-{suffix}")
}

fn note_slug_exists(conn: &Connection, slug: &str, exclude_note_id: Option<&str>) -> Result<bool> {
    let sql = if exclude_note_id.is_some() {
        "SELECT 1 FROM board_notes WHERE note_slug = ?1 AND note_id != ?2 LIMIT 1"
    } else {
        "SELECT 1 FROM board_notes WHERE note_slug = ?1 LIMIT 1"
    };
    let exists = if let Some(note_id) = exclude_note_id {
        conn.query_row(sql, rusqlite::params![slug, note_id], |_| Ok(()))
            .optional()?
            .is_some()
    } else {
        conn.query_row(sql, rusqlite::params![slug], |_| Ok(()))
            .optional()?
            .is_some()
    };
    Ok(exists)
}

fn make_unique_note_slug(
    conn: &Connection,
    note_id: &str,
    body: &str,
    exclude_note_id: Option<&str>,
) -> Result<String> {
    let base = generate_note_slug(note_id, body);
    if !note_slug_exists(conn, &base, exclude_note_id)? {
        return Ok(base);
    }
    for i in 2..=10_000 {
        let candidate = format!("{base}-{i}");
        if !note_slug_exists(conn, &candidate, exclude_note_id)? {
            return Ok(candidate);
        }
    }
    anyhow::bail!("failed to allocate unique note_slug")
}

fn migrate_board_notes_identifiers(conn: &Connection) -> Result<()> {
    ensure_column(conn, "board_notes", "note_slug", "TEXT")?;

    let mut duplicate_stmt = conn.prepare(
        "SELECT note_slug, GROUP_CONCAT(note_id, ', '), COUNT(*)
         FROM board_notes
         WHERE note_slug IS NOT NULL AND TRIM(note_slug) != ''
         GROUP BY note_slug
         HAVING COUNT(*) > 1",
    )?;
    let duplicate_rows = duplicate_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut duplicate_summaries = Vec::new();
    for row in duplicate_rows {
        let (slug, note_ids, count) = row?;
        duplicate_summaries.push(format!("slug={slug} count={count} note_ids=[{note_ids}]"));
    }
    drop(duplicate_stmt);
    if !duplicate_summaries.is_empty() {
        eprintln!(
            "warning: found duplicate board note slugs before migration: {}",
            duplicate_summaries.join("; ")
        );
    }

    let mut stmt = conn
        .prepare("SELECT note_id, note_slug, body FROM board_notes ORDER BY created_at_ms ASC")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut updates = Vec::new();
    for row in rows {
        let (old_note_id, note_slug, body) = row?;
        let new_note_id = match Uuid::parse_str(&old_note_id) {
            Ok(uuid) => uuid.to_string(),
            Err(_) => Uuid::new_v4().to_string(),
        };
        let existing_slug = note_slug
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let slug = if let Some(existing) = existing_slug {
            let cleaned = existing.to_string();
            if note_slug_exists(conn, &cleaned, Some(&old_note_id))? {
                make_unique_note_slug(conn, &new_note_id, &body, Some(&old_note_id))?
            } else {
                cleaned
            }
        } else {
            make_unique_note_slug(conn, &new_note_id, &body, Some(&old_note_id))?
        };
        if new_note_id != old_note_id || note_slug.as_deref() != Some(slug.as_str()) {
            updates.push((old_note_id, new_note_id, slug));
        }
    }
    drop(stmt);

    for (old_note_id, new_note_id, slug) in updates {
        conn.execute(
            "UPDATE board_notes SET note_id = ?2, note_slug = ?3 WHERE note_id = ?1",
            rusqlite::params![old_note_id, new_note_id, slug],
        )?;
    }
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_board_notes_note_slug ON board_notes(note_slug)",
        [],
    )?;
    Ok(())
}

fn init_schema(conn: &Connection, fts5: bool) -> Result<()> {
    conn.execute_batch(SCHEMA_BASE)?;
    crate::memory::init_schema(conn, fts5)?;
    ensure_column(conn, "agent_sessions", "current_workflow", "TEXT")?;
    ensure_column(conn, "agent_sessions", "current_phase", "TEXT")?;
    ensure_column(conn, "agent_sessions", "workflow_status", "TEXT")?;
    ensure_column(conn, "agent_sessions", "current_phase_result", "TEXT")?;
    ensure_column(conn, "agent_sessions", "current_agent", "TEXT")?;
    ensure_column(
        conn,
        "agent_sessions",
        "workflow_last_updated_turn_seq",
        "INTEGER",
    )?;
    ensure_column(conn, "agent_sessions", "workflow_started_at", "INTEGER")?;
    ensure_column(conn, "agent_sessions", "workflow_updated_at", "INTEGER")?;
    ensure_column(conn, "agent_sessions", "workflow_completed_at", "INTEGER")?;
    ensure_column(
        conn,
        "agent_sessions",
        "current_phase_retry_count",
        "INTEGER",
    )?;
    ensure_column(
        conn,
        "agent_sessions",
        "current_phase_retry_limit",
        "INTEGER",
    )?;
    ensure_column(
        conn,
        "agent_sessions",
        "previous_phase_retry_count",
        "INTEGER",
    )?;
    ensure_column(
        conn,
        "agent_sessions",
        "previous_phase_retry_limit",
        "INTEGER",
    )?;
    ensure_column(conn, "agent_sessions", "phase_entered_via", "TEXT")?;
    ensure_column(conn, "agent_turns", "workflow_name", "TEXT")?;
    ensure_column(conn, "agent_turns", "phase_start", "TEXT")?;
    ensure_column(conn, "agent_turns", "phase_end", "TEXT")?;
    ensure_column(conn, "agent_turns", "workflow_status_at_start", "TEXT")?;
    ensure_column(conn, "agent_turns", "phase_result_at_start", "TEXT")?;
    ensure_column(conn, "agent_turns", "workflow_status_at_end", "TEXT")?;
    ensure_column(conn, "agent_turns", "phase_result_at_end", "TEXT")?;
    ensure_column(
        conn,
        "agent_turns",
        "workflow_continues_after_turn",
        "INTEGER",
    )?;
    ensure_column(conn, "agent_turns", "turn_end_reason", "TEXT")?;
    ensure_column(conn, "agent_turns", "meta", "TEXT")?;
    ensure_column(conn, "agent_messages", "workflow_name", "TEXT")?;
    ensure_column(conn, "agent_messages", "phase_name", "TEXT")?;
    ensure_column(
        conn,
        "agent_workflow_transitions",
        "retry_current_count",
        "INTEGER",
    )?;
    ensure_column(
        conn,
        "agent_workflow_transitions",
        "retry_previous_count",
        "INTEGER",
    )?;
    ensure_column(
        conn,
        "agent_workflow_transitions",
        "phase_entered_via",
        "TEXT",
    )?;
    migrate_board_notes_identifiers(conn)?;
    ensure_column(
        conn,
        "board_notes",
        "note_kind",
        "TEXT NOT NULL DEFAULT 'work_request'",
    )?;
    ensure_column(conn, "board_notes", "origin_note_id", "TEXT")?;
    ensure_column(conn, "board_notes", "completion_notified_at_ms", "INTEGER")?;
    ensure_column(conn, "board_notes", "blocked_until_ms", "INTEGER")?;
    ensure_column(conn, "board_notes", "claimed_by_instance", "TEXT")?;
    ensure_column(conn, "board_notes", "claimed_by_agent_id", "TEXT")?;
    ensure_column(conn, "board_notes", "claimed_at_ms", "INTEGER")?;
    ensure_column(conn, "board_notes", "meta_json", "TEXT")?;
    ensure_column(
        conn,
        "scheduled_tasks",
        "timezone",
        "TEXT NOT NULL DEFAULT 'UTC'",
    )?;
    ensure_column(conn, "scheduled_tasks", "retry_after_ms", "INTEGER")?;
    ensure_column(conn, "scheduled_tasks", "last_due_at_ms", "INTEGER")?;
    ensure_column(conn, "scheduled_tasks", "last_run_at_ms", "INTEGER")?;
    ensure_column(
        conn,
        "scheduled_tasks",
        "last_status",
        "TEXT NOT NULL DEFAULT 'never_run'",
    )?;
    ensure_column(conn, "scheduled_tasks", "last_status_detail", "TEXT")?;
    ensure_column(conn, "scheduled_tasks", "last_handling_agent_id", "TEXT")?;
    ensure_column(conn, "scheduled_tasks", "owner_instance", "TEXT")?;
    ensure_column(conn, "scheduled_tasks", "claimed_by_instance", "TEXT")?;
    ensure_column(conn, "scheduled_tasks", "claimed_by_agent_id", "TEXT")?;
    ensure_column(conn, "scheduled_tasks", "claimed_due_at_ms", "INTEGER")?;
    ensure_column(conn, "scheduled_tasks", "claimed_at_ms", "INTEGER")?;
    if fts5 {
        conn.execute_batch(SCHEMA_FTS5)?;
    }
    Ok(())
}

fn sidecar_paths(path: &Path) -> [PathBuf; 2] {
    [
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ]
}

fn migrate_db_file(old_path: &Path, new_path: &Path) -> Result<()> {
    if !old_path.exists() || new_path.exists() {
        return Ok(());
    }

    let rename_attempt = || -> std::io::Result<()> {
        std::fs::rename(old_path, new_path)?;
        for (old_sidecar, new_sidecar) in sidecar_paths(old_path)
            .into_iter()
            .zip(sidecar_paths(new_path))
        {
            if old_sidecar.exists() {
                std::fs::rename(old_sidecar, new_sidecar)?;
            }
        }
        Ok(())
    };

    if rename_attempt().is_ok() {
        return Ok(());
    }

    std::fs::copy(old_path, new_path)?;
    for (old_sidecar, new_sidecar) in sidecar_paths(old_path)
        .into_iter()
        .zip(sidecar_paths(new_path))
    {
        if old_sidecar.exists() {
            std::fs::copy(&old_sidecar, &new_sidecar)?;
        }
    }
    std::fs::remove_file(old_path)?;
    for old_sidecar in sidecar_paths(old_path) {
        if old_sidecar.exists() {
            std::fs::remove_file(old_sidecar)?;
        }
    }
    Ok(())
}

pub fn default_db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("themion").join("system.db")
}

pub fn open_default_in_data_dir(data_dir: &Path) -> Result<Arc<DbHandle>> {
    let themion_dir = data_dir.join("themion");
    let new_path = themion_dir.join("system.db");
    let old_path = themion_dir.join("history.db");

    std::fs::create_dir_all(&themion_dir)?;

    if new_path.exists() {
        if old_path.exists() {
            eprintln!(
                "warning: ignoring legacy database {} because canonical {} already exists",
                old_path.display(),
                new_path.display()
            );
        }
    } else if old_path.exists() {
        migrate_db_file(&old_path, &new_path)?;
    }

    DbHandle::open(new_path)
}

impl DbHandle {
    pub fn open(path: impl AsRef<Path>) -> Result<Arc<Self>> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let fts5 = has_fts5(&conn);
        init_schema(&conn, fts5)?;
        Ok(Arc::new(Self {
            conn: Mutex::new(conn),
            fts5,
        }))
    }

    pub fn open_in_memory() -> Result<Arc<Self>> {
        let conn = Connection::open_in_memory()?;
        let fts5 = has_fts5(&conn);
        init_schema(&conn, fts5)?;
        Ok(Arc::new(Self {
            conn: Mutex::new(conn),
            fts5,
        }))
    }

    pub fn insert_session(
        &self,
        id: uuid::Uuid,
        project_dir: &Path,
        interactive: bool,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO agent_sessions (session_id, project_dir, created_at, is_interactive)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                id.to_string(),
                project_dir.to_string_lossy().as_ref(),
                now_unix(),
                interactive as i64,
            ],
        )?;
        Ok(())
    }

    pub fn update_session_workflow_state(
        &self,
        session_id: uuid::Uuid,
        state: &crate::workflow::WorkflowState,
    ) -> Result<()> {
        let now = now_unix();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE agent_sessions
             SET current_workflow = ?2,
                 current_phase = ?3,
                 workflow_status = ?4,
                 current_phase_result = ?5,
                 current_agent = ?6,
                 workflow_last_updated_turn_seq = ?7,
                 workflow_started_at = COALESCE(workflow_started_at, ?8),
                 workflow_updated_at = ?8,
                 workflow_completed_at = CASE
                     WHEN ?4 IN ('completed', 'failed', 'interrupted') THEN ?8
                     ELSE workflow_completed_at
                 END,
                 current_phase_retry_count = ?9,
                 current_phase_retry_limit = ?10,
                 previous_phase_retry_count = ?11,
                 previous_phase_retry_limit = ?12,
                 phase_entered_via = ?13
             WHERE session_id = ?1",
            rusqlite::params![
                session_id.to_string(),
                state.workflow_name,
                state.phase_name,
                state.status.as_str(),
                state.phase_result.as_str(),
                state.agent_name,
                state.last_updated_turn_seq.map(|v| v as i64),
                now,
                state.retry_state.current_phase_retries as i64,
                state.retry_state.current_phase_retry_limit as i64,
                state.retry_state.previous_phase_retries as i64,
                state.retry_state.previous_phase_retry_limit as i64,
                state.retry_state.entered_via.as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn get_session_workflow_state(
        &self,
        session_id: uuid::Uuid,
    ) -> Result<Option<crate::workflow::WorkflowState>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT current_workflow, current_phase, workflow_status, current_phase_result, current_agent, workflow_last_updated_turn_seq,
                    current_phase_retry_count, current_phase_retry_limit, previous_phase_retry_count,
                    previous_phase_retry_limit, phase_entered_via
             FROM agent_sessions WHERE session_id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![session_id.to_string()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let workflow_name: Option<String> = row.get(0)?;
        let phase_name: Option<String> = row.get(1)?;
        let workflow_status: Option<String> = row.get(2)?;
        let phase_result: Option<String> = row.get(3)?;
        let agent_name: Option<String> = row.get(4)?;
        if workflow_name.is_none() && phase_name.is_none() && workflow_status.is_none() {
            return Ok(None);
        }
        Ok(Some(crate::workflow::WorkflowState {
            workflow_name: workflow_name
                .unwrap_or_else(|| crate::workflow::DEFAULT_WORKFLOW.to_string()),
            phase_name: phase_name.unwrap_or_else(|| crate::workflow::DEFAULT_PHASE.to_string()),
            status: crate::workflow::WorkflowStatus::from_str(
                workflow_status.as_deref().unwrap_or("running"),
            ),
            phase_result: crate::workflow::PhaseResult::from_str(
                phase_result.as_deref().unwrap_or("pending"),
            ),
            agent_name: agent_name.unwrap_or_else(|| crate::workflow::DEFAULT_AGENT.to_string()),
            last_updated_turn_seq: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
            retry_state: crate::workflow::PhaseRetryState {
                current_phase_retries: row.get::<_, Option<i64>>(6)?.unwrap_or(0) as u32,
                current_phase_retry_limit: row
                    .get::<_, Option<i64>>(7)?
                    .unwrap_or(crate::workflow::MAX_CURRENT_PHASE_RETRIES as i64)
                    as u32,
                previous_phase_retries: row.get::<_, Option<i64>>(8)?.unwrap_or(0) as u32,
                previous_phase_retry_limit: row
                    .get::<_, Option<i64>>(9)?
                    .unwrap_or(crate::workflow::MAX_PREVIOUS_PHASE_RETRIES as i64)
                    as u32,
                entered_via: crate::workflow::PhaseEntryKind::from_str(
                    row.get::<_, Option<String>>(10)?
                        .as_deref()
                        .unwrap_or("normal"),
                ),
            },
        }))
    }

    pub fn begin_turn(
        &self,
        session_id: uuid::Uuid,
        turn_seq: u32,
        workflow: &crate::workflow::WorkflowState,
        turn_meta: Option<&TurnMeta>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_turns (
                session_id, turn_seq, created_at, workflow_name, phase_start, workflow_status_at_start, phase_result_at_start
             , meta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                session_id.to_string(),
                turn_seq as i64,
                now_unix(),
                workflow.workflow_name,
                workflow.phase_name,
                workflow.status.as_str(),
                workflow.phase_result.as_str(),
                turn_meta.and_then(|meta| meta.to_json().ok()).flatten(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_turn(&self, turn_id: i64) -> Result<Option<TurnRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT turn_id, session_id, turn_seq, meta FROM agent_turns WHERE turn_id = ?1",
            rusqlite::params![turn_id],
            map_turn_row,
        )
        .optional()
        .map_err(anyhow::Error::from)
    }

    pub fn append_message(
        &self,
        turn_id: i64,
        session_id: uuid::Uuid,
        seq: u32,
        msg: &crate::client::Message,
        workflow: &crate::workflow::WorkflowState,
    ) -> Result<i64> {
        let tool_calls_json = match &msg.tool_calls {
            Some(tc) => Some(serde_json::to_string(tc)?),
            None => None,
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_messages (
                turn_id, session_id, seq, role, content, tool_calls_json, tool_call_id, workflow_name, phase_name
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                turn_id,
                session_id.to_string(),
                seq as i64,
                msg.role,
                msg.content,
                tool_calls_json,
                msg.tool_call_id,
                workflow.workflow_name,
                workflow.phase_name,
            ],
        )?;
        let message_id = conn.last_insert_rowid();
        let project_dir: String = conn.query_row(
            "SELECT project_dir FROM agent_sessions WHERE session_id = ?1",
            rusqlite::params![session_id.to_string()],
            |row| row.get(0),
        )?;
        let created_at_s: i64 = conn.query_row(
            "SELECT created_at FROM agent_turns WHERE turn_id = ?1",
            rusqlite::params![turn_id],
            |row| row.get(0),
        )?;
        drop(conn);
        let _ = self
            .memory_store()
            .register_appended_chat_message_for_unified_search(AppendedChatMessageIndexArgs {
                message_id,
                session_id: session_id.to_string(),
                turn_seq: seq,
                role: msg.role.clone(),
                content: msg.content.clone(),
                tool_calls_json,
                project_dir,
                created_at_s,
            });
        Ok(message_id)
    }

    pub fn record_workflow_transition(
        &self,
        session_id: uuid::Uuid,
        turn_id: Option<i64>,
        turn_seq: Option<u32>,
        workflow_name: &str,
        from_phase: Option<&str>,
        to_phase: &str,
        workflow_status: &str,
        transition_kind: crate::workflow::WorkflowTransitionKind,
        trigger_source: Option<&str>,
        message_id: Option<i64>,
        retry_current_count: Option<u32>,
        retry_previous_count: Option<u32>,
        phase_entered_via: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_workflow_transitions (
                session_id, turn_id, turn_seq, workflow_name, from_phase, to_phase,
                workflow_status, transition_kind, trigger_source, message_id,
                retry_current_count, retry_previous_count, phase_entered_via, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                session_id.to_string(),
                turn_id,
                turn_seq.map(|v| v as i64),
                workflow_name,
                from_phase,
                to_phase,
                workflow_status,
                transition_kind.as_str(),
                trigger_source,
                message_id,
                retry_current_count.map(|v| v as i64),
                retry_previous_count.map(|v| v as i64),
                phase_entered_via,
                now_unix(),
            ],
        )?;
        Ok(())
    }

    pub fn finalize_turn(
        &self,
        turn_id: i64,
        stats: &crate::agent::TurnStats,
        workflow: &crate::workflow::WorkflowState,
        workflow_continues_after_turn: bool,
        turn_end_reason: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE agent_turns
             SET tokens_in = ?1, tokens_out = ?2, tokens_cached = ?3,
                 llm_rounds = ?4, tool_calls_count = ?5,
                 phase_end = ?6,
                 workflow_status_at_end = ?7,
                 phase_result_at_end = ?8,
                 workflow_continues_after_turn = ?9,
                 turn_end_reason = ?10
             WHERE turn_id = ?11",
            rusqlite::params![
                stats.tokens_in as i64,
                stats.tokens_out as i64,
                stats.tokens_cached as i64,
                stats.llm_rounds as i64,
                stats.tool_calls as i64,
                workflow.phase_name,
                workflow.status.as_str(),
                workflow.phase_result.as_str(),
                workflow_continues_after_turn as i64,
                turn_end_reason,
                turn_id,
            ],
        )?;
        Ok(())
    }

    pub fn recall(&self, args: RecallArgs) -> Result<Vec<RecalledMessage>> {
        let limit = args.limit.min(200) as i64;
        let conn = self.conn.lock().unwrap();
        let project_str = args.current_project_dir.to_string_lossy().into_owned();

        let order = match args.direction {
            RecallDirection::Newest => "DESC",
            RecallDirection::Oldest => "ASC",
        };

        let mut out = Vec::new();
        match args.session_scope {
            SessionScope::Current => {
                anyhow::bail!("SessionScope::Current must be resolved before db recall");
            }
            SessionScope::AllInCurrentProject => {
                let sql = format!(
                    "SELECT m.session_id, t.turn_seq, m.role, m.content, m.tool_calls_json, m.tool_call_id
                     FROM agent_messages m
                     JOIN agent_turns t ON m.turn_id = t.turn_id
                     JOIN agent_sessions s ON m.session_id = s.session_id
                     WHERE s.project_dir = ?1
                     ORDER BY s.created_at {order}, t.turn_seq {order}, m.seq {order}
                     LIMIT ?2"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params![project_str, limit], |row| {
                    Ok(RecalledMessage {
                        session_id: row.get(0)?,
                        turn_seq: row.get::<_, i64>(1)? as u32,
                        role: row.get(2)?,
                        content: row.get(3)?,
                        tool_calls_json: row.get(4)?,
                        tool_call_id: row.get(5)?,
                    })
                })?;
                for r in rows {
                    out.push(r?);
                }
            }
            SessionScope::Exact(session_id) => {
                let sql = format!(
                    "SELECT m.session_id, t.turn_seq, m.role, m.content, m.tool_calls_json, m.tool_call_id
                     FROM agent_messages m
                     JOIN agent_turns t ON m.turn_id = t.turn_id
                     JOIN agent_sessions s ON m.session_id = s.session_id
                     WHERE m.session_id = ?1 AND s.project_dir = ?2
                     ORDER BY t.turn_seq {order}, m.seq {order}
                     LIMIT ?3"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(
                    rusqlite::params![session_id.to_string(), project_str, limit],
                    |row| {
                        Ok(RecalledMessage {
                            session_id: row.get(0)?,
                            turn_seq: row.get::<_, i64>(1)? as u32,
                            role: row.get(2)?,
                            content: row.get(3)?,
                            tool_calls_json: row.get(4)?,
                            tool_call_id: row.get(5)?,
                        })
                    },
                )?;
                for r in rows {
                    out.push(r?);
                }
            }
        }
        Ok(out)
    }

    pub fn create_board_note(&self, args: CreateNoteArgs) -> Result<BoardNote> {
        let now_ms = now_unix_ms();
        let note_id = Uuid::parse_str(&args.note_id)
            .map(|uuid| uuid.to_string())
            .unwrap_or_else(|_| Uuid::new_v4().to_string());
        let conn = self.conn.lock().unwrap();
        let note_slug = make_unique_note_slug(&conn, &note_id, &args.body, None)?;
        let note = BoardNote {
            note_id: note_id.clone(),
            note_slug: note_slug.clone(),
            note_kind: args.note_kind,
            origin_note_id: args.origin_note_id,
            completion_notified_at_ms: None,
            from_instance: args.from_instance,
            from_agent_id: args.from_agent_id,
            to_instance: args.to_instance,
            to_agent_id: args.to_agent_id,
            body: args.body,
            column: args.column,
            result_text: None,
            injection_state: NoteInjectionState::Pending,
            blocked_until_ms: (args.column == NoteColumn::Blocked)
                .then(|| now_ms + BLOCKED_RETRY_COOLDOWN_MS),
            claimed_by_instance: None,
            claimed_by_agent_id: None,
            claimed_at_ms: None,
            meta_json: args.meta_json,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            injected_at_ms: None,
        };
        warn_if_unexpected_board_note_instance(&note.to_instance, "to_instance", &note.note_id);
        if let Some(from_instance) = note.from_instance.as_deref() {
            warn_if_unexpected_board_note_instance(from_instance, "from_instance", &note.note_id);
        }
        conn.execute(
            "INSERT INTO board_notes (
                note_id, note_slug, note_kind, origin_note_id, completion_notified_at_ms,
                from_instance, from_agent_id, to_instance, to_agent_id, body, column_name, result_text,
                injection_state, blocked_until_ms, claimed_by_instance, claimed_by_agent_id, claimed_at_ms, meta_json, created_at_ms, updated_at_ms, injected_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
            rusqlite::params![
                note.note_id,
                note.note_slug,
                note.note_kind.as_str(),
                note.origin_note_id,
                note.completion_notified_at_ms,
                note.from_instance,
                note.from_agent_id,
                note.to_instance,
                note.to_agent_id,
                note.body,
                note.column.as_str(),
                note.result_text,
                note.injection_state.as_str(),
                note.blocked_until_ms,
                note.claimed_by_instance,
                note.claimed_by_agent_id,
                note.claimed_at_ms,
                note.meta_json,
                note.created_at_ms,
                note.updated_at_ms,
                note.injected_at_ms,
            ],
        )?;
        drop(conn);
        self.get_board_note(&note_id)?
            .ok_or_else(|| anyhow::anyhow!("note insert failed"))
    }

    pub fn get_board_note(&self, note_id: &str) -> Result<Option<BoardNote>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT note_id, note_slug, note_kind, origin_note_id, completion_notified_at_ms, from_instance, from_agent_id, to_instance, to_agent_id, body,
                    column_name, result_text, injection_state, blocked_until_ms, claimed_by_instance, claimed_by_agent_id, claimed_at_ms,
                    meta_json, created_at_ms, updated_at_ms, injected_at_ms
             FROM board_notes WHERE note_id = ?1",
            rusqlite::params![note_id],
            map_note_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_board_notes(
        &self,
        to_instance: Option<&str>,
        to_agent_id: Option<&str>,
        columns: &[NoteColumn],
    ) -> Result<Vec<BoardNote>> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT note_id, note_slug, note_kind, origin_note_id, completion_notified_at_ms, from_instance, from_agent_id, to_instance, to_agent_id, body,
                    column_name, result_text, injection_state, blocked_until_ms, claimed_by_instance, claimed_by_agent_id, claimed_at_ms,
                    meta_json, created_at_ms, updated_at_ms, injected_at_ms
             FROM board_notes
             WHERE (?1 IS NULL OR to_instance = ?1)
               AND (?2 IS NULL OR to_agent_id = ?2)",
        );
        if !columns.is_empty() {
            sql.push_str(
                "
               AND column_name IN (",
            );
            for i in 0..columns.len() {
                if i > 0 {
                    sql.push_str(", ");
                }
                sql.push('?');
                sql.push_str(&(i + 3).to_string());
            }
            sql.push(')');
        }
        sql.push_str(
            "
             ORDER BY created_at_ms ASC",
        );

        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(2 + columns.len());
        params.push(&to_instance);
        params.push(&to_agent_id);
        let column_names: Vec<&str> = columns.iter().map(|c| c.as_str()).collect();
        for name in &column_names {
            params.push(name);
        }

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), map_note_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn update_board_note(
        &self,
        note_id: &str,
        column: Option<NoteColumn>,
        result_text: Option<Option<&str>>,
    ) -> Result<Option<BoardNote>> {
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();

        match (column, result_text) {
            (Some(column), Some(result_text)) => {
                conn.execute(
                    "UPDATE board_notes
                     SET column_name = ?2,
                         blocked_until_ms = CASE WHEN ?2 = 'blocked' THEN ?3 ELSE NULL END,
                         injection_state = CASE WHEN ?2 = 'done' THEN injection_state ELSE 'pending' END,
                         claimed_by_instance = NULL,
                         claimed_by_agent_id = NULL,
                         claimed_at_ms = NULL,
                         result_text = ?4,
                         updated_at_ms = ?5
                     WHERE note_id = ?1",
                    rusqlite::params![
                        note_id,
                        column.as_str(),
                        now_ms + BLOCKED_RETRY_COOLDOWN_MS,
                        result_text,
                        now_ms
                    ],
                )?;
            }
            (Some(column), None) => {
                conn.execute(
                    "UPDATE board_notes
                     SET column_name = ?2,
                         blocked_until_ms = CASE WHEN ?2 = 'blocked' THEN ?3 ELSE NULL END,
                         injection_state = CASE WHEN ?2 = 'done' THEN injection_state ELSE 'pending' END,
                         claimed_by_instance = NULL,
                         claimed_by_agent_id = NULL,
                         claimed_at_ms = NULL,
                         updated_at_ms = ?4
                     WHERE note_id = ?1",
                    rusqlite::params![
                        note_id,
                        column.as_str(),
                        now_ms + BLOCKED_RETRY_COOLDOWN_MS,
                        now_ms
                    ],
                )?;
            }
            (None, Some(result_text)) => {
                conn.execute(
                    "UPDATE board_notes SET result_text = ?2, updated_at_ms = ?3 WHERE note_id = ?1",
                    rusqlite::params![note_id, result_text, now_ms],
                )?;
            }
            (None, None) => return self.get_board_note(note_id),
        }

        drop(conn);
        self.get_board_note(note_id)
    }

    pub fn next_board_note_for_injection(
        &self,
        to_instance: &str,
        to_agent_id: &str,
    ) -> Result<Option<BoardNote>> {
        let conn = self.conn.lock().unwrap();
        let in_progress = conn
            .query_row(
                "SELECT note_id, note_slug, note_kind, origin_note_id, completion_notified_at_ms, from_instance, from_agent_id, to_instance, to_agent_id, body,
                        column_name, result_text, injection_state, blocked_until_ms, claimed_by_instance, claimed_by_agent_id, claimed_at_ms,
                        meta_json, created_at_ms, updated_at_ms, injected_at_ms
                 FROM board_notes
                 WHERE to_instance = ?1 AND to_agent_id = ?2 AND injection_state = 'pending'
                   AND column_name = 'in_progress'
                 ORDER BY created_at_ms ASC
                 LIMIT 1",
                rusqlite::params![to_instance, to_agent_id],
                map_note_row,
            )
            .optional()?;
        if in_progress.is_some() {
            return Ok(in_progress);
        }
        conn.query_row(
            "SELECT note_id, note_slug, note_kind, origin_note_id, completion_notified_at_ms, from_instance, from_agent_id, to_instance, to_agent_id, body,
                    column_name, result_text, injection_state, blocked_until_ms, claimed_by_instance, claimed_by_agent_id, claimed_at_ms,
                    meta_json, created_at_ms, updated_at_ms, injected_at_ms
             FROM board_notes
             WHERE to_instance = ?1 AND to_agent_id = ?2 AND injection_state = 'pending'
               AND column_name = 'todo'
             ORDER BY created_at_ms ASC
             LIMIT 1",
            rusqlite::params![to_instance, to_agent_id],
            map_note_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn next_shared_board_note_candidate(&self, to_instance: &str) -> Result<Option<BoardNote>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT note_id, note_slug, note_kind, origin_note_id, completion_notified_at_ms, from_instance, from_agent_id, to_instance, to_agent_id, body,
                    column_name, result_text, injection_state, blocked_until_ms, claimed_by_instance, claimed_by_agent_id, claimed_at_ms,
                    meta_json, created_at_ms, updated_at_ms, injected_at_ms
             FROM board_notes
             WHERE to_instance = ?1 AND to_agent_id = '[any]'
               AND column_name = 'todo'
               AND injection_state = 'pending'
               AND claimed_by_instance IS NULL
               AND claimed_by_agent_id IS NULL
               AND claimed_at_ms IS NULL
             ORDER BY created_at_ms ASC
             LIMIT 1",
            rusqlite::params![to_instance],
            map_note_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn claim_shared_board_note(
        &self,
        note_id: &str,
        claimed_by_instance: &str,
        claimed_by_agent_id: &str,
    ) -> Result<bool> {
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE board_notes
             SET claimed_by_instance = ?2,
                 claimed_by_agent_id = ?3,
                 claimed_at_ms = ?4,
                 updated_at_ms = ?4
             WHERE note_id = ?1
               AND to_agent_id = '[any]'
               AND column_name = 'todo'
               AND injection_state = 'pending'
               AND claimed_by_instance IS NULL
               AND claimed_by_agent_id IS NULL
               AND claimed_at_ms IS NULL",
            rusqlite::params![note_id, claimed_by_instance, claimed_by_agent_id, now_ms],
        )?;
        Ok(changed == 1)
    }

    pub fn release_shared_board_note_claim(&self, note_id: &str) -> Result<()> {
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE board_notes
             SET claimed_by_instance = NULL, claimed_by_agent_id = NULL, claimed_at_ms = NULL, updated_at_ms = ?2
             WHERE note_id = ?1",
            rusqlite::params![note_id, now_ms],
        )?;
        Ok(())
    }

    pub fn mark_board_note_completion_notified(&self, note_id: &str) -> Result<Option<BoardNote>> {
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE board_notes
             SET completion_notified_at_ms = COALESCE(completion_notified_at_ms, ?2), updated_at_ms = ?2
             WHERE note_id = ?1",
            rusqlite::params![note_id, now_ms],
        )?;
        drop(conn);
        self.get_board_note(note_id)
    }

    pub fn mark_board_note_injected(&self, note_id: &str) -> Result<Option<BoardNote>> {
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE board_notes
             SET injection_state = 'injected',
                 injected_at_ms = ?2,
                 blocked_until_ms = CASE WHEN column_name = 'blocked' THEN ?3 ELSE blocked_until_ms END,
                 claimed_by_instance = NULL,
                 claimed_by_agent_id = NULL,
                 claimed_at_ms = NULL,
                 updated_at_ms = ?2
             WHERE note_id = ?1",
            rusqlite::params![note_id, now_ms, now_ms + BLOCKED_RETRY_COOLDOWN_MS],
        )?;
        drop(conn);
        self.get_board_note(note_id)
    }

    pub fn create_scheduled_task(&self, args: CreateScheduledTaskArgs) -> Result<ScheduledTask> {
        args.validate()?;
        let now_ms = now_unix_ms();
        let schedule_id = args
            .schedule_id
            .as_deref()
            .and_then(|raw| Uuid::parse_str(raw).ok())
            .map(|uuid| uuid.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let task = ScheduledTask {
            schedule_id: schedule_id.clone(),
            title: args.title,
            body: args.body,
            enabled: args.enabled,
            schedule_kind: args.schedule_kind,
            due_at_ms: args.due_at_ms,
            cron_expr: args.cron_expr,
            timezone: "UTC".to_string(),
            next_due_at_ms: args.next_due_at_ms,
            target_agent_id: args.target_agent_id,
            owner_instance: args.owner_instance,
            busy_policy: args.busy_policy,
            retry_after_ms: None,
            last_due_at_ms: None,
            last_run_at_ms: None,
            last_status: ScheduledTaskStatus::NeverRun,
            last_status_detail: None,
            last_handling_agent_id: None,
            claimed_by_instance: None,
            claimed_by_agent_id: None,
            claimed_due_at_ms: None,
            claimed_at_ms: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO scheduled_tasks (
                schedule_id, title, body, enabled, schedule_kind, due_at_ms, cron_expr, timezone,
                next_due_at_ms, target_agent_id, owner_instance, busy_policy, retry_after_ms,
                last_due_at_ms, last_run_at_ms, last_status, last_status_detail,
                last_handling_agent_id, claimed_by_instance, claimed_by_agent_id,
                claimed_due_at_ms, claimed_at_ms, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            rusqlite::params![
                task.schedule_id,
                task.title,
                task.body,
                task.enabled as i64,
                task.schedule_kind.as_str(),
                task.due_at_ms,
                task.cron_expr,
                task.timezone,
                task.next_due_at_ms,
                task.target_agent_id,
                task.owner_instance,
                task.busy_policy.as_str(),
                task.retry_after_ms,
                task.last_due_at_ms,
                task.last_run_at_ms,
                task.last_status.as_str(),
                task.last_status_detail,
                task.last_handling_agent_id,
                task.claimed_by_instance,
                task.claimed_by_agent_id,
                task.claimed_due_at_ms,
                task.claimed_at_ms,
                task.created_at_ms,
                task.updated_at_ms,
            ],
        )?;
        drop(conn);
        self.get_scheduled_task(&schedule_id)?
            .ok_or_else(|| anyhow::anyhow!("scheduled task insert failed"))
    }

    pub fn get_scheduled_task(&self, schedule_id: &str) -> Result<Option<ScheduledTask>> {
        let conn = self.conn.lock().unwrap();
        let sql = SCHEDULED_TASK_SELECT_SQL.to_string() + " WHERE schedule_id = ?1";
        conn.query_row(&sql, rusqlite::params![schedule_id], map_scheduled_task_row)
            .optional()
            .map_err(Into::into)
    }

    pub fn list_scheduled_tasks(&self, args: ListScheduledTasksArgs) -> Result<Vec<ScheduledTask>> {
        let conn = self.conn.lock().unwrap();
        let mut sql = SCHEDULED_TASK_SELECT_SQL.to_string();
        sql.push_str(" WHERE (?1 IS NULL OR enabled = ?1)");
        match &args.target_agent_id {
            Some(Some(_)) => sql.push_str(" AND target_agent_id = ?2"),
            Some(None) => sql.push_str(" AND target_agent_id IS NULL"),
            None => sql.push_str(" AND (?2 IS NULL OR 1 = 1)"),
        }
        if args.due_only {
            sql.push_str(" AND next_due_at_ms IS NOT NULL AND next_due_at_ms <= ?3");
        } else {
            sql.push_str(" AND (?3 IS NULL OR 1 = 1)");
        }
        sql.push_str(
            " ORDER BY next_due_at_ms IS NULL, next_due_at_ms ASC, created_at_ms ASC LIMIT ?4",
        );
        let enabled = args.enabled.map(|v| v as i64);
        let now = now_unix_ms();
        let limit = args.limit.unwrap_or(50).clamp(1, 200) as i64;
        let mut stmt = conn.prepare(&sql)?;
        let target_agent_id = args.target_agent_id.flatten();
        let rows = stmt.query_map(
            rusqlite::params![enabled, target_agent_id, now, limit],
            map_scheduled_task_row,
        )?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn update_scheduled_task(
        &self,
        schedule_id: &str,
        patch: UpdateScheduledTaskArgs,
    ) -> Result<Option<ScheduledTask>> {
        let Some(mut task) = self.get_scheduled_task(schedule_id)? else {
            return Ok(None);
        };
        if let Some(title) = patch.title {
            task.title = title;
        }
        if let Some(body) = patch.body {
            task.body = body;
        }
        if let Some(enabled) = patch.enabled {
            task.enabled = enabled;
        }
        if let Some(schedule_kind) = patch.schedule_kind {
            task.schedule_kind = schedule_kind;
            task.due_at_ms = patch.due_at_ms;
            task.cron_expr = patch.cron_expr;
            task.next_due_at_ms = patch.next_due_at_ms;
        } else {
            if patch.due_at_ms.is_some() {
                task.due_at_ms = patch.due_at_ms;
                task.next_due_at_ms = patch.next_due_at_ms.or(patch.due_at_ms);
            }
            if patch.cron_expr.is_some() {
                task.cron_expr = patch.cron_expr;
                task.next_due_at_ms = patch.next_due_at_ms;
            }
        }
        if patch.target_agent_id_set {
            task.target_agent_id = patch.target_agent_id;
        }
        if let Some(busy_policy) = patch.busy_policy {
            task.busy_policy = busy_policy;
        }
        task.validate()?;
        task.updated_at_ms = now_unix_ms();
        task.retry_after_ms = None;
        task.claimed_by_instance = None;
        task.claimed_by_agent_id = None;
        task.claimed_due_at_ms = None;
        task.claimed_at_ms = None;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_tasks
             SET title = ?2, body = ?3, enabled = ?4, schedule_kind = ?5, due_at_ms = ?6,
                 cron_expr = ?7, timezone = ?8, next_due_at_ms = ?9, target_agent_id = ?10,
                 busy_policy = ?11, retry_after_ms = NULL, claimed_by_instance = NULL,
                 claimed_by_agent_id = NULL, claimed_due_at_ms = NULL, claimed_at_ms = NULL,
                 updated_at_ms = ?12
             WHERE schedule_id = ?1",
            rusqlite::params![
                task.schedule_id,
                task.title,
                task.body,
                task.enabled as i64,
                task.schedule_kind.as_str(),
                task.due_at_ms,
                task.cron_expr,
                task.timezone,
                task.next_due_at_ms,
                task.target_agent_id,
                task.busy_policy.as_str(),
                task.updated_at_ms,
            ],
        )?;
        drop(conn);
        self.get_scheduled_task(schedule_id)
    }

    pub fn delete_scheduled_task(&self, schedule_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "DELETE FROM scheduled_tasks WHERE schedule_id = ?1",
            rusqlite::params![schedule_id],
        )?;
        Ok(changed > 0)
    }

    pub fn next_due_scheduled_task(
        &self,
        now_ms: i64,
        owner_instance: &str,
    ) -> Result<Option<ScheduledTask>> {
        let conn = self.conn.lock().unwrap();
        let sql = SCHEDULED_TASK_SELECT_SQL.to_string()
            + " WHERE enabled = 1
                AND next_due_at_ms IS NOT NULL
                AND next_due_at_ms <= ?1
                AND (retry_after_ms IS NULL OR retry_after_ms <= ?1)
                AND (owner_instance = ?2 OR owner_instance IS NULL)
                AND claimed_at_ms IS NULL
              ORDER BY next_due_at_ms ASC, created_at_ms ASC
              LIMIT 1";
        conn.query_row(
            &sql,
            rusqlite::params![now_ms, owner_instance],
            map_scheduled_task_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn claim_scheduled_task(
        &self,
        schedule_id: &str,
        due_at_ms: i64,
        owner_instance: &str,
        claimed_by_instance: &str,
        claimed_by_agent_id: &str,
    ) -> Result<bool> {
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE scheduled_tasks
             SET last_status = 'claimed', owner_instance = ?6, claimed_by_instance = ?3, claimed_by_agent_id = ?4,
                 claimed_due_at_ms = ?2, claimed_at_ms = ?5, updated_at_ms = ?5
             WHERE schedule_id = ?1
               AND enabled = 1
               AND next_due_at_ms = ?2
               AND (owner_instance = ?6 OR owner_instance IS NULL)
               AND claimed_at_ms IS NULL",
            rusqlite::params![
                schedule_id,
                due_at_ms,
                claimed_by_instance,
                claimed_by_agent_id,
                now_ms,
                owner_instance
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn finish_scheduled_task_occurrence(
        &self,
        args: FinishScheduledTaskOccurrenceArgs,
    ) -> Result<Option<ScheduledTask>> {
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_tasks
             SET enabled = ?2, next_due_at_ms = ?3, retry_after_ms = ?4, last_due_at_ms = ?5,
                 last_run_at_ms = ?6, last_status = ?7, last_status_detail = ?8,
                 last_handling_agent_id = ?9, claimed_by_instance = NULL, claimed_by_agent_id = NULL,
                 claimed_due_at_ms = NULL, claimed_at_ms = NULL, updated_at_ms = ?6
             WHERE schedule_id = ?1",
            rusqlite::params![
                args.schedule_id,
                args.enabled as i64,
                args.next_due_at_ms,
                args.retry_after_ms,
                args.due_at_ms,
                now_ms,
                args.status.as_str(),
                args.status_detail,
                args.handling_agent_id,
            ],
        )?;
        drop(conn);
        self.get_scheduled_task(&args.schedule_id)
    }

    pub fn memory_store(&self) -> crate::memory::MemoryStore<'_> {
        crate::memory::MemoryStore::new(&self.conn, self.fts5)
    }

    pub fn search(&self, args: SearchArgs) -> Result<Vec<SearchHit>> {
        if !self.fts5 {
            return Ok(vec![]);
        }
        let limit = args.limit.min(100) as i64;
        let conn = self.conn.lock().unwrap();
        let project_str = args.current_project_dir.to_string_lossy().into_owned();
        let mut out = Vec::new();

        match args.session_scope {
            SessionScope::Current => {
                anyhow::bail!("SessionScope::Current must be resolved before db recall");
            }
            SessionScope::AllInCurrentProject => {
                let sql = "SELECT m.session_id, t.turn_seq, m.role,
                        snippet(agent_messages_fts, 0, '**', '**', '...', 16)
                 FROM agent_messages_fts
                 JOIN agent_messages m ON agent_messages_fts.rowid = m.message_id
                 JOIN agent_turns t ON m.turn_id = t.turn_id
                 JOIN agent_sessions s ON m.session_id = s.session_id
                 WHERE agent_messages_fts MATCH ?1
                   AND s.project_dir = ?2
                 LIMIT ?3";
                let mut stmt = conn.prepare(sql)?;
                let rows =
                    stmt.query_map(rusqlite::params![args.query, project_str, limit], |row| {
                        Ok(SearchHit {
                            session_id: row.get(0)?,
                            turn_seq: row.get::<_, i64>(1)? as u32,
                            role: row.get(2)?,
                            snippet: row.get(3)?,
                        })
                    })?;
                for r in rows {
                    out.push(r?);
                }
            }
            SessionScope::Exact(session_id) => {
                let sql = "SELECT m.session_id, t.turn_seq, m.role,
                        snippet(agent_messages_fts, 0, '**', '**', '...', 16)
                 FROM agent_messages_fts
                 JOIN agent_messages m ON agent_messages_fts.rowid = m.message_id
                 JOIN agent_turns t ON m.turn_id = t.turn_id
                 JOIN agent_sessions s ON m.session_id = s.session_id
                 WHERE agent_messages_fts MATCH ?1
                   AND m.session_id = ?2
                   AND s.project_dir = ?3
                 LIMIT ?4";
                let mut stmt = conn.prepare(sql)?;
                let rows = stmt.query_map(
                    rusqlite::params![args.query, session_id.to_string(), project_str, limit],
                    |row| {
                        Ok(SearchHit {
                            session_id: row.get(0)?,
                            turn_seq: row.get::<_, i64>(1)? as u32,
                            role: row.get(2)?,
                            snippet: row.get(3)?,
                        })
                    },
                )?;
                for r in rows {
                    out.push(r?);
                }
            }
        }
        Ok(out)
    }

    pub fn unified_search(
        &self,
        query: crate::memory::UnifiedSearchQuery,
        default_project_dir: Option<&std::path::Path>,
    ) -> Result<crate::memory::UnifiedSearchResponse> {
        use crate::memory::{
            append_unified_search_rows, memory_search_to_unified, SearchNodesArgs,
            UnifiedSearchMode, UnifiedSearchResponse, UnifiedSearchSourceKind,
        };

        let default_project_dir =
            default_project_dir.map(|path| path.to_string_lossy().to_string());
        let normalized = query.normalize(default_project_dir.as_deref())?;
        let wants_memory = normalized
            .source_kinds
            .iter()
            .any(|kind| matches!(kind, UnifiedSearchSourceKind::Memory));
        let wants_db_rows = normalized.source_kinds.iter().any(|kind| {
            matches!(
                kind,
                UnifiedSearchSourceKind::ChatMessage
                    | UnifiedSearchSourceKind::ToolCall
                    | UnifiedSearchSourceKind::ToolResult
            )
        });

        let mut response = UnifiedSearchResponse {
            mode: normalized.mode,
            degraded: false,
            degradation_reason: None,
            pending_index_count: 0,
            unavailable_source_kinds: Vec::new(),
            results: Vec::new(),
        };

        if matches!(
            normalized.mode,
            UnifiedSearchMode::Semantic | UnifiedSearchMode::Hybrid
        ) && !normalized.query.is_empty()
        {
            #[cfg(feature = "semantic-memory")]
            {
                let requested_source_kinds = normalized
                    .source_kinds
                    .iter()
                    .map(|kind| kind.as_str().to_string())
                    .collect::<Vec<_>>();
                let mut semantic = self.memory_store().unified_search_semantic(
                    &normalized.project_dir,
                    &requested_source_kinds,
                    &normalized.query,
                    normalized.limit,
                )?;
                if matches!(normalized.mode, UnifiedSearchMode::Hybrid) {
                    semantic.mode = UnifiedSearchMode::Hybrid;
                }
                response = semantic;
            }
            #[cfg(not(feature = "semantic-memory"))]
            {
                response.degraded = true;
                response.degradation_reason = Some(
                    "semantic retrieval unavailable: themion was built without the semantic-memory feature"
                        .to_string(),
                );
            }
        } else if wants_memory {
            let nodes = self.memory_store().search_nodes(SearchNodesArgs {
                query: Some(normalized.query.clone()),
                project_dir: normalized.project_dir.clone(),
                hashtags: normalized.hashtags.clone(),
                hashtag_match: normalized.hashtag_match,
                node_type: normalized.node_type.clone(),
                relation_type: normalized.relation_type.clone(),
                linked_node_id: normalized.linked_node_id.clone(),
                limit: normalized.limit,
                mode: normalized.mode,
            })?;
            response = memory_search_to_unified(nodes);
            response.mode = normalized.mode;
        }

        if matches!(
            normalized.mode,
            UnifiedSearchMode::Fts | UnifiedSearchMode::Hybrid
        ) && wants_db_rows
            && !normalized.query.is_empty()
        {
            let allowed_source_kinds = normalized
                .source_kinds
                .iter()
                .map(|kind| kind.as_str().to_string())
                .collect::<std::collections::BTreeSet<_>>();
            let mut rows = self.unified_search_rows(crate::db::SearchArgs {
                query: normalized.query.clone(),
                session_scope: crate::db::SessionScope::AllInCurrentProject,
                current_project_dir: std::path::PathBuf::from(normalized.project_dir.clone()),
                limit: normalized.limit,
            })?;
            rows.retain(|row| allowed_source_kinds.contains(&row.source_kind));
            if matches!(normalized.mode, UnifiedSearchMode::Hybrid) {
                let exact_ids = rows
                    .iter()
                    .map(|row| {
                        (
                            row.source_kind.clone(),
                            row.source_id.clone(),
                            row.project_dir.clone(),
                        )
                    })
                    .collect::<std::collections::BTreeSet<_>>();
                for result in response.results.iter_mut() {
                    if exact_ids.contains(&(
                        result.source_kind.clone(),
                        result.source_id.clone(),
                        result.project_dir.clone(),
                    )) {
                        result.score += 0.15 * result.score;
                        result.score_kind = UnifiedSearchMode::Hybrid;
                    }
                }
                let already = response
                    .results
                    .iter()
                    .map(|result| {
                        (
                            result.source_kind.clone(),
                            result.source_id.clone(),
                            result.project_dir.clone(),
                        )
                    })
                    .collect::<std::collections::BTreeSet<_>>();
                rows.retain(|row| {
                    !already.contains(&(
                        row.source_kind.clone(),
                        row.source_id.clone(),
                        row.project_dir.clone(),
                    ))
                });
                append_unified_search_rows(&mut response, rows, UnifiedSearchMode::Fts);
            } else {
                append_unified_search_rows(&mut response, rows, normalized.mode);
            }
        }

        response.results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        response.results.truncate(normalized.limit as usize);

        let _ = normalized.source_kinds_were_omitted;
        Ok(response)
    }

    pub fn unified_search_rows(&self, args: SearchArgs) -> Result<Vec<UnifiedSearchSourceRow>> {
        if !self.fts5 {
            return Ok(vec![]);
        }
        let limit = args.limit.min(100) as i64;
        let conn = self.conn.lock().unwrap();
        let project_str = args.current_project_dir.to_string_lossy().into_owned();
        let mut out = Vec::new();
        let sql = "SELECT m.message_id, m.session_id, t.turn_seq, m.role, m.content, m.tool_calls_json, m.tool_call_id
             FROM agent_messages_fts
             JOIN agent_messages m ON agent_messages_fts.rowid = m.message_id
             JOIN agent_turns t ON m.turn_id = t.turn_id
             JOIN agent_sessions s ON m.session_id = s.session_id
             WHERE agent_messages_fts MATCH ?1
               AND s.project_dir = ?2
             LIMIT ?3";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params![args.query, project_str, limit], |row| {
            let message_id: i64 = row.get(0)?;
            let session_id: String = row.get(1)?;
            let turn_seq: u32 = row.get::<_, i64>(2)? as u32;
            let role: String = row.get(3)?;
            let content: Option<String> = row.get(4)?;
            let tool_calls_json: Option<String> = row.get(5)?;
            let _tool_call_id: Option<String> = row.get(6)?;
            let snippet = content.clone().unwrap_or_default();
            let (source_kind, source_id, title) = if role == "tool" {
                let tool_name = content
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .and_then(|value| {
                        value
                            .get("tool_name")
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "tool_result".to_string());
                (
                    "tool_result".to_string(),
                    format!("{}:{}", session_id, message_id),
                    tool_name,
                )
            } else if tool_calls_json
                .as_deref()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
            {
                let tool_name = serde_json::from_str::<serde_json::Value>(
                    tool_calls_json.as_deref().unwrap_or("[]"),
                )
                .ok()
                .and_then(|value| value.as_array().and_then(|arr| arr.first().cloned()))
                .and_then(|value| {
                    value
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "tool_call".to_string());
                (
                    "tool_call".to_string(),
                    format!("{}:{}", session_id, message_id),
                    tool_name,
                )
            } else {
                let source_kind = if role == "user" || role == "assistant" || role == "system" {
                    "chat_message"
                } else {
                    "chat_message"
                };
                let title = role.clone();
                (
                    source_kind.to_string(),
                    format!("{}:{}", session_id, message_id),
                    title,
                )
            };
            Ok(UnifiedSearchSourceRow {
                source_kind,
                source_id,
                project_dir: project_str.clone(),
                session_id: Some(session_id),
                turn_seq: Some(turn_seq),
                role: Some(role),
                title,
                snippet,
            })
        })?;
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

const SCHEDULED_TASK_SELECT_SQL: &str = "SELECT schedule_id, title, body, enabled, schedule_kind,
       due_at_ms, cron_expr, timezone, next_due_at_ms, target_agent_id, busy_policy,
       retry_after_ms, last_due_at_ms, last_run_at_ms, last_status, last_status_detail,
       last_handling_agent_id, COALESCE(owner_instance, ''), claimed_by_instance, claimed_by_agent_id,
       claimed_due_at_ms, claimed_at_ms, created_at_ms, updated_at_ms FROM scheduled_tasks";

fn map_scheduled_task_row(row: &Row<'_>) -> rusqlite::Result<ScheduledTask> {
    let schedule_kind_raw: String = row.get(4)?;
    let busy_policy_raw: String = row.get(10)?;
    let last_status_raw: String = row.get(14)?;
    Ok(ScheduledTask {
        schedule_id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        enabled: row.get::<_, i64>(3)? != 0,
        schedule_kind: ScheduledKind::from_str(&schedule_kind_raw)
            .unwrap_or(ScheduledKind::OneShot),
        due_at_ms: row.get(5)?,
        cron_expr: row.get(6)?,
        timezone: row.get(7)?,
        next_due_at_ms: row.get(8)?,
        target_agent_id: row.get(9)?,
        busy_policy: ScheduleBusyPolicy::from_str(&busy_policy_raw)
            .unwrap_or(ScheduleBusyPolicy::Delay),
        retry_after_ms: row.get(11)?,
        last_due_at_ms: row.get(12)?,
        last_run_at_ms: row.get(13)?,
        last_status: ScheduledTaskStatus::from_str(&last_status_raw)
            .unwrap_or(ScheduledTaskStatus::Failed),
        last_status_detail: row.get(15)?,
        last_handling_agent_id: row.get(16)?,
        owner_instance: row.get(17)?,
        claimed_by_instance: row.get(18)?,
        claimed_by_agent_id: row.get(19)?,
        claimed_due_at_ms: row.get(20)?,
        claimed_at_ms: row.get(21)?,
        created_at_ms: row.get(22)?,
        updated_at_ms: row.get(23)?,
    })
}

fn map_note_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BoardNote> {
    let note_kind_raw: String = row.get(2)?;
    let column_raw: String = row.get(10)?;
    let injection_raw: String = row.get(12)?;
    Ok(BoardNote {
        note_id: row.get(0)?,
        note_slug: row.get(1)?,
        note_kind: NoteKind::from_str(&note_kind_raw).ok_or_else(|| {
            rusqlite::Error::InvalidColumnType(2, "note_kind".into(), rusqlite::types::Type::Text)
        })?,
        origin_note_id: row.get(3)?,
        completion_notified_at_ms: row.get(4)?,
        from_instance: row.get(5)?,
        from_agent_id: row.get(6)?,
        to_instance: row.get(7)?,
        to_agent_id: row.get(8)?,
        body: row.get(9)?,
        column: NoteColumn::from_str(&column_raw).ok_or_else(|| {
            rusqlite::Error::InvalidColumnType(
                10,
                "column_name".into(),
                rusqlite::types::Type::Text,
            )
        })?,
        result_text: row.get(11)?,
        injection_state: NoteInjectionState::from_str(&injection_raw).ok_or_else(|| {
            rusqlite::Error::InvalidColumnType(
                12,
                "injection_state".into(),
                rusqlite::types::Type::Text,
            )
        })?,
        blocked_until_ms: row.get(13)?,
        claimed_by_instance: row.get(14)?,
        claimed_by_agent_id: row.get(15)?,
        claimed_at_ms: row.get(16)?,
        meta_json: row.get(17)?,
        created_at_ms: row.get(18)?,
        updated_at_ms: row.get(19)?,
        injected_at_ms: row.get(20)?,
    })
}

pub struct RecallArgs {
    pub session_scope: SessionScope,
    pub current_project_dir: std::path::PathBuf,
    pub limit: u32,
    pub direction: RecallDirection,
}

pub enum RecallDirection {
    Newest,
    Oldest,
}

pub struct RecalledMessage {
    pub session_id: String,
    pub turn_seq: u32,
    pub role: String,
    pub content: Option<String>,
    pub tool_calls_json: Option<String>,
    pub tool_call_id: Option<String>,
}

pub struct SearchArgs {
    pub query: String,
    pub session_scope: SessionScope,
    pub current_project_dir: std::path::PathBuf,
    pub limit: u32,
}

#[derive(Clone, Copy)]
pub enum SessionScope {
    Current,
    Exact(uuid::Uuid),
    AllInCurrentProject,
}

pub struct SearchHit {
    pub session_id: String,
    pub turn_seq: u32,
    pub role: String,
    pub snippet: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UnifiedSearchSourceRow {
    pub source_kind: String,
    pub source_id: String,
    pub project_dir: String,
    pub session_id: Option<String>,
    pub turn_seq: Option<u32>,
    pub role: Option<String>,
    pub title: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteColumn {
    Todo,
    InProgress,
    Blocked,
    Done,
}

impl NoteColumn {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "todo" => Some(Self::Todo),
            "in_progress" => Some(Self::InProgress),
            "blocked" => Some(Self::Blocked),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteInjectionState {
    Pending,
    Injected,
}

impl NoteInjectionState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Injected => "injected",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "injected" => Some(Self::Injected),
            _ => None,
        }
    }
}

fn warn_if_unexpected_board_note_instance(instance: &str, field_name: &str, note_id: &str) {
    if instance.is_empty() {
        eprintln!(
            "warning: board note {} created with empty {}",
            note_id, field_name
        );
        return;
    }
    if instance == "local" {
        eprintln!(
            "warning: board note {} created with legacy {}=local; expected concrete instance id like <hostname>:<pid>",
            note_id, field_name
        );
        return;
    }
    if instance == "[any]" {
        return;
    }
    if !instance.contains(':') {
        eprintln!(
            "warning: board note {} created with unexpected {} format {:?}; expected <hostname>:<pid>",
            note_id, field_name, instance
        );
    }
}

#[derive(Debug, Clone)]
pub struct BoardNote {
    pub note_id: String,
    pub note_slug: String,
    pub note_kind: NoteKind,
    pub origin_note_id: Option<String>,
    pub completion_notified_at_ms: Option<i64>,
    pub from_instance: Option<String>,
    pub from_agent_id: Option<String>,
    pub to_instance: String,
    pub to_agent_id: String,
    pub body: String,
    pub column: NoteColumn,
    pub result_text: Option<String>,
    pub injection_state: NoteInjectionState,
    pub blocked_until_ms: Option<i64>,
    pub claimed_by_instance: Option<String>,
    pub claimed_by_agent_id: Option<String>,
    pub claimed_at_ms: Option<i64>,
    pub meta_json: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub injected_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CreateNoteArgs {
    pub note_id: String,
    pub note_kind: NoteKind,
    pub column: NoteColumn,
    pub origin_note_id: Option<String>,
    pub from_instance: Option<String>,
    pub from_agent_id: Option<String>,
    pub to_instance: String,
    pub to_agent_id: String,
    pub body: String,
    pub meta_json: Option<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum ScheduledKind {
    OneShot,
    Cron,
}

impl ScheduledKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OneShot => "one_shot",
            Self::Cron => "cron",
        }
    }

    pub fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "one_shot" => Some(Self::OneShot),
            "cron" => Some(Self::Cron),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum ScheduleBusyPolicy {
    Delay,
    DelegateToFreeAgent,
    CreateNewAgent,
    Miss,
}

impl ScheduleBusyPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delay => "delay",
            Self::DelegateToFreeAgent => "delegate_to_free_agent",
            Self::CreateNewAgent => "create_new_agent",
            Self::Miss => "miss",
        }
    }

    pub fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "delay" => Some(Self::Delay),
            "delegate_to_free_agent" => Some(Self::DelegateToFreeAgent),
            "create_new_agent" => Some(Self::CreateNewAgent),
            "miss" => Some(Self::Miss),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum ScheduledTaskStatus {
    NeverRun,
    Claimed,
    Dispatched,
    Delayed,
    Delegated,
    Missed,
    Failed,
}

impl ScheduledTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NeverRun => "never_run",
            Self::Claimed => "claimed",
            Self::Dispatched => "dispatched",
            Self::Delayed => "delayed",
            Self::Delegated => "delegated",
            Self::Missed => "missed",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "never_run" => Some(Self::NeverRun),
            "claimed" => Some(Self::Claimed),
            "dispatched" => Some(Self::Dispatched),
            "delayed" => Some(Self::Delayed),
            "delegated" => Some(Self::Delegated),
            "missed" => Some(Self::Missed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ScheduledTask {
    pub schedule_id: String,
    pub title: String,
    pub body: String,
    pub enabled: bool,
    pub schedule_kind: ScheduledKind,
    pub due_at_ms: Option<i64>,
    pub cron_expr: Option<String>,
    pub timezone: String,
    pub next_due_at_ms: Option<i64>,
    pub target_agent_id: Option<String>,
    pub owner_instance: String,
    pub busy_policy: ScheduleBusyPolicy,
    pub retry_after_ms: Option<i64>,
    pub last_due_at_ms: Option<i64>,
    pub last_run_at_ms: Option<i64>,
    pub last_status: ScheduledTaskStatus,
    pub last_status_detail: Option<String>,
    pub last_handling_agent_id: Option<String>,
    pub claimed_by_instance: Option<String>,
    pub claimed_by_agent_id: Option<String>,
    pub claimed_due_at_ms: Option<i64>,
    pub claimed_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl ScheduledTask {
    fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            anyhow::bail!("title cannot be empty");
        }
        if self.body.trim().is_empty() {
            anyhow::bail!("body cannot be empty");
        }
        if self.timezone != "UTC" {
            anyhow::bail!("only timezone='UTC' is supported");
        }
        match self.schedule_kind {
            ScheduledKind::OneShot => {
                if self.due_at_ms.is_none() || self.next_due_at_ms.is_none() {
                    anyhow::bail!("one_shot schedules require due_at_ms");
                }
            }
            ScheduledKind::Cron => {
                if self
                    .cron_expr
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    anyhow::bail!("cron schedules require cron_expr");
                }
                if self.next_due_at_ms.is_none() {
                    anyhow::bail!("cron schedules require next_due_at_ms");
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CreateScheduledTaskArgs {
    pub schedule_id: Option<String>,
    pub title: String,
    pub body: String,
    pub enabled: bool,
    pub schedule_kind: ScheduledKind,
    pub due_at_ms: Option<i64>,
    pub cron_expr: Option<String>,
    pub next_due_at_ms: Option<i64>,
    pub target_agent_id: Option<String>,
    pub owner_instance: String,
    pub busy_policy: ScheduleBusyPolicy,
}

impl CreateScheduledTaskArgs {
    fn validate(&self) -> Result<()> {
        ScheduledTask {
            schedule_id: self.schedule_id.clone().unwrap_or_default(),
            title: self.title.clone(),
            body: self.body.clone(),
            enabled: self.enabled,
            schedule_kind: self.schedule_kind,
            due_at_ms: self.due_at_ms,
            cron_expr: self.cron_expr.clone(),
            timezone: "UTC".to_string(),
            next_due_at_ms: self.next_due_at_ms,
            target_agent_id: self.target_agent_id.clone(),
            owner_instance: self.owner_instance.clone(),
            busy_policy: self.busy_policy,
            retry_after_ms: None,
            last_due_at_ms: None,
            last_run_at_ms: None,
            last_status: ScheduledTaskStatus::NeverRun,
            last_status_detail: None,
            last_handling_agent_id: None,
            claimed_by_instance: None,
            claimed_by_agent_id: None,
            claimed_due_at_ms: None,
            claimed_at_ms: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
        .validate()
    }
}

#[derive(Debug, Clone, Default)]
pub struct UpdateScheduledTaskArgs {
    pub title: Option<String>,
    pub body: Option<String>,
    pub enabled: Option<bool>,
    pub schedule_kind: Option<ScheduledKind>,
    pub due_at_ms: Option<i64>,
    pub cron_expr: Option<String>,
    pub next_due_at_ms: Option<i64>,
    pub target_agent_id_set: bool,
    pub target_agent_id: Option<String>,
    pub busy_policy: Option<ScheduleBusyPolicy>,
}

#[derive(Debug, Clone, Default)]
pub struct ListScheduledTasksArgs {
    pub enabled: Option<bool>,
    pub target_agent_id: Option<Option<String>>,
    pub due_only: bool,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct FinishScheduledTaskOccurrenceArgs {
    pub schedule_id: String,
    pub due_at_ms: i64,
    pub enabled: bool,
    pub next_due_at_ms: Option<i64>,
    pub retry_after_ms: Option<i64>,
    pub status: ScheduledTaskStatus,
    pub status_detail: Option<String>,
    pub handling_agent_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    WorkRequest,
    DoneMention,
}

impl NoteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorkRequest => "work_request",
            Self::DoneMention => "done_mention",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "work_request" => Some(Self::WorkRequest),
            "done_mention" => Some(Self::DoneMention),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_meta_round_trip_and_legacy_null_are_supported() {
        let db = DbHandle::open_in_memory().expect("db");
        let session_id = uuid::Uuid::new_v4();
        db.insert_session(session_id, Path::new("/tmp"), true)
            .expect("session");
        let workflow = crate::workflow::WorkflowState::default();

        let with_meta = TurnMeta {
            app_version: Some("0.36.0".to_string()),
            app_version_hash: Some("abc1234".to_string()),
            app_version_dirty: Some(false),
            profile: Some("codex".to_string()),
            provider: Some("openai-codex".to_string()),
            model: Some("gpt-5.4".to_string()),
        };
        let turn_id = db
            .begin_turn(session_id, 1, &workflow, Some(&with_meta))
            .expect("turn");
        let row = db.get_turn(turn_id).expect("read").expect("row");
        assert_eq!(row.meta, Some(with_meta));
        assert!(row
            .meta_json
            .as_deref()
            .unwrap_or_default()
            .contains("provider"));

        let legacy_turn_id = db
            .begin_turn(session_id, 2, &workflow, None)
            .expect("legacy turn");
        let legacy_row = db
            .get_turn(legacy_turn_id)
            .expect("read legacy")
            .expect("legacy row");
        assert_eq!(legacy_row.meta, None);
        assert_eq!(legacy_row.meta_json, None);
    }

    #[test]
    fn scheduled_task_claim_is_atomic_and_finish_clears_claim() {
        let db = DbHandle::open_in_memory().expect("db");
        let task = db
            .create_scheduled_task(CreateScheduledTaskArgs {
                schedule_id: Some("00000000-0000-0000-0000-000000000001".to_string()),
                title: "demo".to_string(),
                body: "run demo".to_string(),
                enabled: true,
                schedule_kind: ScheduledKind::OneShot,
                due_at_ms: Some(1_000),
                cron_expr: None,
                next_due_at_ms: Some(1_000),
                target_agent_id: Some("master".to_string()),
                owner_instance: "host-a:100".to_string(),
                busy_policy: ScheduleBusyPolicy::Delay,
            })
            .expect("create schedule");

        assert!(db
            .next_due_scheduled_task(1_000, "host-b:200")
            .unwrap()
            .is_none());
        assert_eq!(
            db.next_due_scheduled_task(1_000, "host-a:100")
                .unwrap()
                .unwrap()
                .schedule_id,
            task.schedule_id
        );
        assert!(!db
            .claim_scheduled_task(&task.schedule_id, 1_000, "host-b:200", "local", "master")
            .expect("wrong owner claim loses"));

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE scheduled_tasks SET owner_instance = NULL WHERE schedule_id = ?1",
                rusqlite::params![task.schedule_id],
            )
            .unwrap();
        }
        assert_eq!(
            db.next_due_scheduled_task(1_000, "host-b:200")
                .unwrap()
                .unwrap()
                .owner_instance,
            ""
        );
        assert!(db
            .claim_scheduled_task(&task.schedule_id, 1_000, "host-a:100", "local", "master")
            .expect("legacy unowned schedule can be claimed"));
        assert!(!db
            .claim_scheduled_task(&task.schedule_id, 1_000, "host-b:200", "local", "other")
            .expect("legacy schedule claim is bound after first claim"));
        assert!(!db
            .claim_scheduled_task(&task.schedule_id, 1_000, "host-a:100", "local", "other")
            .expect("second claim loses"));

        let claimed = db
            .get_scheduled_task(&task.schedule_id)
            .expect("read claimed")
            .expect("claimed task");
        assert_eq!(claimed.last_status, ScheduledTaskStatus::Claimed);
        assert_eq!(claimed.claimed_by_agent_id.as_deref(), Some("master"));

        let finished = db
            .finish_scheduled_task_occurrence(FinishScheduledTaskOccurrenceArgs {
                schedule_id: task.schedule_id.clone(),
                due_at_ms: 1_000,
                enabled: false,
                next_due_at_ms: None,
                retry_after_ms: None,
                status: ScheduledTaskStatus::Dispatched,
                status_detail: None,
                handling_agent_id: Some("master".to_string()),
            })
            .expect("finish")
            .expect("finished task");

        assert!(!finished.enabled);
        assert_eq!(finished.last_status, ScheduledTaskStatus::Dispatched);
        assert_eq!(finished.last_handling_agent_id.as_deref(), Some("master"));
        assert_eq!(finished.claimed_at_ms, None);
        assert_eq!(finished.claimed_by_agent_id, None);
    }
}
