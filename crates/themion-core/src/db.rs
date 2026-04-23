use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const BLOCKED_RETRY_COOLDOWN_MS: i64 = 5 * 60 * 1000;

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
    turn_end_reason TEXT
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
    ensure_column(conn, "board_notes", "meta_json", "TEXT")?;
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
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_turns (
                session_id, turn_seq, created_at, workflow_name, phase_start, workflow_status_at_start, phase_result_at_start
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                session_id.to_string(),
                turn_seq as i64,
                now_unix(),
                workflow.workflow_name,
                workflow.phase_name,
                workflow.status.as_str(),
                workflow.phase_result.as_str(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
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
        Ok(conn.last_insert_rowid())
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

        let mut filters = Vec::new();
        if args.session_id.is_some() {
            filters.push("m.session_id = ?1");
        }
        if args.project_dir.is_some() {
            filters.push("s.project_dir = ?2");
        }
        let where_clause = if filters.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", filters.join(" AND "))
        };

        let order = match args.direction {
            RecallDirection::Newest => "DESC",
            RecallDirection::Oldest => "ASC",
        };

        let sql = format!(
            "SELECT t.turn_seq, m.role, m.content, m.tool_calls_json, m.tool_call_id
             FROM agent_messages m
             JOIN agent_turns t ON m.turn_id = t.turn_id
             JOIN agent_sessions s ON m.session_id = s.session_id
             {where_clause}
             ORDER BY t.turn_seq {order}, m.seq {order}
             LIMIT ?3"
        );

        let session_str = args.session_id.map(|u| u.to_string()).unwrap_or_default();
        let project_str = args
            .project_dir
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![session_str, project_str, limit], |row| {
            Ok(RecalledMessage {
                turn_seq: row.get::<_, i64>(0)? as u32,
                role: row.get(1)?,
                content: row.get(2)?,
                tool_calls_json: row.get(3)?,
                tool_call_id: row.get(4)?,
            })
        })?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
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
            meta_json: args.meta_json,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            injected_at_ms: None,
        };
        conn.execute(
            "INSERT INTO board_notes (
                note_id, note_slug, note_kind, origin_note_id, completion_notified_at_ms,
                from_instance, from_agent_id, to_instance, to_agent_id, body,
                column_name, result_text, injection_state, blocked_until_ms, meta_json, created_at_ms, updated_at_ms, injected_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
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
                    column_name, result_text, injection_state, blocked_until_ms, meta_json, created_at_ms, updated_at_ms, injected_at_ms
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
        column: Option<NoteColumn>,
    ) -> Result<Vec<BoardNote>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT note_id, note_slug, note_kind, origin_note_id, completion_notified_at_ms, from_instance, from_agent_id, to_instance, to_agent_id, body,
                    column_name, result_text, injection_state, blocked_until_ms, meta_json, created_at_ms, updated_at_ms, injected_at_ms
             FROM board_notes
             WHERE (?1 IS NULL OR to_instance = ?1)
               AND (?2 IS NULL OR to_agent_id = ?2)
               AND (?3 IS NULL OR column_name = ?3)
             ORDER BY created_at_ms ASC",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![to_instance, to_agent_id, column.map(|c| c.as_str())],
            map_note_row,
        )?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn move_board_note(&self, note_id: &str, column: NoteColumn) -> Result<Option<BoardNote>> {
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE board_notes
             SET column_name = ?2,
                 blocked_until_ms = CASE WHEN ?2 = 'blocked' THEN ?3 ELSE NULL END,
                 injection_state = CASE WHEN ?2 = 'done' THEN injection_state ELSE 'pending' END,
                 updated_at_ms = ?4
             WHERE note_id = ?1",
            rusqlite::params![
                note_id,
                column.as_str(),
                now_ms + BLOCKED_RETRY_COOLDOWN_MS,
                now_ms
            ],
        )?;
        drop(conn);
        self.get_board_note(note_id)
    }

    pub fn update_board_note_result(
        &self,
        note_id: &str,
        result_text: Option<&str>,
    ) -> Result<Option<BoardNote>> {
        let now_ms = now_unix_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE board_notes SET result_text = ?2, updated_at_ms = ?3 WHERE note_id = ?1",
            rusqlite::params![note_id, result_text, now_ms],
        )?;
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
                        column_name, result_text, injection_state, blocked_until_ms, meta_json, created_at_ms, updated_at_ms, injected_at_ms
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
                    column_name, result_text, injection_state, blocked_until_ms, meta_json, created_at_ms, updated_at_ms, injected_at_ms
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
                 updated_at_ms = ?2
             WHERE note_id = ?1",
            rusqlite::params![note_id, now_ms, now_ms + BLOCKED_RETRY_COOLDOWN_MS],
        )?;
        drop(conn);
        self.get_board_note(note_id)
    }

    pub fn search(&self, args: SearchArgs) -> Result<Vec<SearchHit>> {
        if !self.fts5 {
            return Ok(vec![]);
        }
        let limit = args.limit.min(100) as i64;
        let conn = self.conn.lock().unwrap();

        let mut extra_filters = Vec::new();
        if args.session_id.is_some() {
            extra_filters.push("m.session_id = ?2");
        }
        if args.project_dir.is_some() {
            extra_filters.push("s.project_dir = ?3");
        }
        let extra_where = if extra_filters.is_empty() {
            String::new()
        } else {
            format!("AND {}", extra_filters.join(" AND "))
        };

        let sql = format!(
            "SELECT m.session_id, t.turn_seq, m.role,
                    snippet(agent_messages_fts, 0, '**', '**', '...', 16)
             FROM agent_messages_fts
             JOIN agent_messages m ON agent_messages_fts.rowid = m.message_id
             JOIN agent_turns t ON m.turn_id = t.turn_id
             JOIN agent_sessions s ON m.session_id = s.session_id
             WHERE agent_messages_fts MATCH ?1
             {extra_where}
             LIMIT ?4"
        );

        let session_str = args.session_id.map(|u| u.to_string()).unwrap_or_default();
        let project_str = args
            .project_dir
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params![args.query, session_str, project_str, limit],
            |row| {
                Ok(SearchHit {
                    session_id: row.get(0)?,
                    turn_seq: row.get::<_, i64>(1)? as u32,
                    role: row.get(2)?,
                    snippet: row.get(3)?,
                })
            },
        )?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
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
        meta_json: row.get(14)?,
        created_at_ms: row.get(15)?,
        updated_at_ms: row.get(16)?,
        injected_at_ms: row.get(17)?,
    })
}

pub struct RecallArgs {
    pub session_id: Option<uuid::Uuid>,
    pub project_dir: Option<std::path::PathBuf>,
    pub limit: u32,
    pub direction: RecallDirection,
}

pub enum RecallDirection {
    Newest,
    Oldest,
}

pub struct RecalledMessage {
    pub turn_seq: u32,
    pub role: String,
    pub content: Option<String>,
    pub tool_calls_json: Option<String>,
    pub tool_call_id: Option<String>,
}

pub struct SearchArgs {
    pub query: String,
    pub session_id: Option<uuid::Uuid>,
    pub project_dir: Option<std::path::PathBuf>,
    pub limit: u32,
}

pub struct SearchHit {
    pub session_id: String,
    pub turn_seq: u32,
    pub role: String,
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
