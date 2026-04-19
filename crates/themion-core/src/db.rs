use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
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
    current_agent TEXT,
    workflow_last_updated_turn_seq INTEGER,
    workflow_started_at INTEGER,
    workflow_updated_at INTEGER,
    workflow_completed_at INTEGER
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
    workflow_status_at_end TEXT,
    workflow_continues_after_turn INTEGER,
    turn_end_reason TEXT,
    UNIQUE(session_id, turn_seq)
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
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_workflow_transitions_session_time
    ON agent_workflow_transitions(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_agent_workflow_transitions_turn
    ON agent_workflow_transitions(turn_id);
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
    let found = rows.filter_map(|r| r.ok()).any(|o| o == "ENABLE_FTS5");
    found
}

fn ensure_column(conn: &Connection, table: &str, col: &str, decl: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let exists = cols.filter_map(|r| r.ok()).any(|name| name == col);
    if !exists {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {col} {decl}"),
            [],
        )?;
    }
    Ok(())
}

fn init_schema(conn: &Connection, fts5: bool) -> Result<()> {
    conn.execute_batch(SCHEMA_BASE)?;
    ensure_column(conn, "agent_sessions", "current_workflow", "TEXT")?;
    ensure_column(conn, "agent_sessions", "current_phase", "TEXT")?;
    ensure_column(conn, "agent_sessions", "workflow_status", "TEXT")?;
    ensure_column(conn, "agent_sessions", "current_agent", "TEXT")?;
    ensure_column(conn, "agent_sessions", "workflow_last_updated_turn_seq", "INTEGER")?;
    ensure_column(conn, "agent_sessions", "workflow_started_at", "INTEGER")?;
    ensure_column(conn, "agent_sessions", "workflow_updated_at", "INTEGER")?;
    ensure_column(conn, "agent_sessions", "workflow_completed_at", "INTEGER")?;
    ensure_column(conn, "agent_turns", "workflow_name", "TEXT")?;
    ensure_column(conn, "agent_turns", "phase_start", "TEXT")?;
    ensure_column(conn, "agent_turns", "phase_end", "TEXT")?;
    ensure_column(conn, "agent_turns", "workflow_status_at_start", "TEXT")?;
    ensure_column(conn, "agent_turns", "workflow_status_at_end", "TEXT")?;
    ensure_column(conn, "agent_turns", "workflow_continues_after_turn", "INTEGER")?;
    ensure_column(conn, "agent_turns", "turn_end_reason", "TEXT")?;
    ensure_column(conn, "agent_messages", "workflow_name", "TEXT")?;
    ensure_column(conn, "agent_messages", "phase_name", "TEXT")?;
    if fts5 {
        conn.execute_batch(SCHEMA_FTS5)?;
    }
    Ok(())
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
                 current_agent = ?5,
                 workflow_last_updated_turn_seq = ?6,
                 workflow_started_at = COALESCE(workflow_started_at, ?7),
                 workflow_updated_at = ?7,
                 workflow_completed_at = CASE
                     WHEN ?4 IN ('completed', 'failed', 'interrupted') THEN ?7
                     ELSE workflow_completed_at
                 END
             WHERE session_id = ?1",
            rusqlite::params![
                session_id.to_string(),
                state.workflow_name,
                state.phase_name,
                state.status.as_str(),
                state.agent_name,
                state.last_updated_turn_seq.map(|v| v as i64),
                now,
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
            "SELECT current_workflow, current_phase, workflow_status, current_agent, workflow_last_updated_turn_seq
             FROM agent_sessions WHERE session_id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![session_id.to_string()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let workflow_name: Option<String> = row.get(0)?;
        let phase_name: Option<String> = row.get(1)?;
        let workflow_status: Option<String> = row.get(2)?;
        let agent_name: Option<String> = row.get(3)?;
        if workflow_name.is_none() && phase_name.is_none() && workflow_status.is_none() {
            return Ok(None);
        }
        Ok(Some(crate::workflow::WorkflowState {
            workflow_name: workflow_name.unwrap_or_else(|| crate::workflow::DEFAULT_WORKFLOW.to_string()),
            phase_name: phase_name.unwrap_or_else(|| crate::workflow::DEFAULT_PHASE.to_string()),
            status: crate::workflow::WorkflowStatus::from_str(
                workflow_status.as_deref().unwrap_or("running"),
            ),
            agent_name: agent_name.unwrap_or_else(|| crate::workflow::DEFAULT_AGENT.to_string()),
            last_updated_turn_seq: row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
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
                session_id, turn_seq, created_at, workflow_name, phase_start, workflow_status_at_start
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                session_id.to_string(),
                turn_seq as i64,
                now_unix(),
                workflow.workflow_name,
                workflow.phase_name,
                workflow.status.as_str(),
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
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agent_workflow_transitions (
                session_id, turn_id, turn_seq, workflow_name, from_phase, to_phase,
                workflow_status, transition_kind, trigger_source, message_id, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                 workflow_continues_after_turn = ?8,
                 turn_end_reason = ?9
             WHERE turn_id = ?10",
            rusqlite::params![
                stats.tokens_in as i64,
                stats.tokens_out as i64,
                stats.tokens_cached as i64,
                stats.llm_rounds as i64,
                stats.tool_calls as i64,
                workflow.phase_name,
                workflow.status.as_str(),
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
