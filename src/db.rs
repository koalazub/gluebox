use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct SpecMapping {
    pub linear_issue_id: String,
    pub anytype_object_id: Option<String>,
    pub linear_url: Option<String>,
    pub anytype_url: Option<String>,
    pub last_synced_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FeedbackTicket {
    pub linear_issue_id: String,
    pub linear_issue_url: String,
    pub title: String,
    pub category: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ContractMapping {
    pub documenso_document_id: String,
    pub anytype_object_id: Option<String>,
    pub linear_issue_id: Option<String>,
    pub status: Option<String>,
    pub last_synced_at: Option<String>,
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let db = Db { conn: Mutex::new(conn) };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS spec_mappings (
                linear_issue_id TEXT PRIMARY KEY,
                anytype_object_id TEXT,
                linear_url TEXT,
                anytype_url TEXT,
                last_synced_at TEXT
            );

            CREATE TABLE IF NOT EXISTS contract_mappings (
                documenso_document_id TEXT PRIMARY KEY,
                anytype_object_id TEXT,
                linear_issue_id TEXT,
                status TEXT,
                last_synced_at TEXT
            );

            CREATE TABLE IF NOT EXISTS event_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                event_type TEXT NOT NULL,
                external_id TEXT NOT NULL,
                payload TEXT,
                processed_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS feedback_tickets (
                linear_issue_id TEXT PRIMARY KEY,
                linear_issue_url TEXT NOT NULL,
                title TEXT NOT NULL,
                category TEXT NOT NULL,
                description TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_spec_anytype ON spec_mappings(anytype_object_id);
            CREATE INDEX IF NOT EXISTS idx_contract_anytype ON contract_mappings(anytype_object_id);
            CREATE INDEX IF NOT EXISTS idx_contract_linear ON contract_mappings(linear_issue_id);
            CREATE INDEX IF NOT EXISTS idx_event_log_ext ON event_log(source, external_id);
            CREATE INDEX IF NOT EXISTS idx_feedback_category ON feedback_tickets(category);"
        )?;
        Ok(())
    }

    pub fn upsert_spec(&self, mapping: &SpecMapping) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO spec_mappings (linear_issue_id, anytype_object_id, linear_url, anytype_url, last_synced_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(linear_issue_id) DO UPDATE SET
                anytype_object_id = COALESCE(excluded.anytype_object_id, anytype_object_id),
                linear_url = COALESCE(excluded.linear_url, linear_url),
                anytype_url = COALESCE(excluded.anytype_url, anytype_url),
                last_synced_at = datetime('now')",
            params![
                mapping.linear_issue_id,
                mapping.anytype_object_id,
                mapping.linear_url,
                mapping.anytype_url,
            ],
        )?;
        Ok(())
    }

    pub fn get_spec_by_linear_id(&self, linear_issue_id: &str) -> anyhow::Result<Option<SpecMapping>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT linear_issue_id, anytype_object_id, linear_url, anytype_url, last_synced_at
             FROM spec_mappings WHERE linear_issue_id = ?1"
        )?;
        let result = stmt.query_row(params![linear_issue_id], |row| {
            Ok(SpecMapping {
                linear_issue_id: row.get(0)?,
                anytype_object_id: row.get(1)?,
                linear_url: row.get(2)?,
                anytype_url: row.get(3)?,
                last_synced_at: row.get(4)?,
            })
        });
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_spec_by_anytype_id(&self, anytype_object_id: &str) -> anyhow::Result<Option<SpecMapping>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT linear_issue_id, anytype_object_id, linear_url, anytype_url, last_synced_at
             FROM spec_mappings WHERE anytype_object_id = ?1"
        )?;
        let result = stmt.query_row(params![anytype_object_id], |row| {
            Ok(SpecMapping {
                linear_issue_id: row.get(0)?,
                anytype_object_id: row.get(1)?,
                linear_url: row.get(2)?,
                anytype_url: row.get(3)?,
                last_synced_at: row.get(4)?,
            })
        });
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn upsert_contract(&self, mapping: &ContractMapping) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO contract_mappings (documenso_document_id, anytype_object_id, linear_issue_id, status, last_synced_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(documenso_document_id) DO UPDATE SET
                anytype_object_id = COALESCE(excluded.anytype_object_id, anytype_object_id),
                linear_issue_id = COALESCE(excluded.linear_issue_id, linear_issue_id),
                status = COALESCE(excluded.status, status),
                last_synced_at = datetime('now')",
            params![
                mapping.documenso_document_id,
                mapping.anytype_object_id,
                mapping.linear_issue_id,
                mapping.status,
            ],
        )?;
        Ok(())
    }

    pub fn get_contract_by_documenso_id(&self, doc_id: &str) -> anyhow::Result<Option<ContractMapping>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT documenso_document_id, anytype_object_id, linear_issue_id, status, last_synced_at
             FROM contract_mappings WHERE documenso_document_id = ?1"
        )?;
        let result = stmt.query_row(params![doc_id], |row| {
            Ok(ContractMapping {
                documenso_document_id: row.get(0)?,
                anytype_object_id: row.get(1)?,
                linear_issue_id: row.get(2)?,
                status: row.get(3)?,
                last_synced_at: row.get(4)?,
            })
        });
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn log_event(&self, source: &str, event_type: &str, external_id: &str, payload: Option<&str>) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO event_log (source, event_type, external_id, payload) VALUES (?1, ?2, ?3, ?4)",
            params![source, event_type, external_id, payload],
        )?;
        Ok(())
    }

    pub fn insert_feedback_ticket(
        &self,
        linear_issue_id: &str,
        linear_issue_url: &str,
        title: &str,
        category: &str,
        description: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO feedback_tickets
             (linear_issue_id, linear_issue_url, title, category, description)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![linear_issue_id, linear_issue_url, title, category, description],
        )?;
        Ok(())
    }

    /// Return all feedback tickets in the same category, up to `limit`.
    /// Used for deduplication: compare new cluster against these before creating.
    pub fn get_feedback_by_category(
        &self,
        category: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<FeedbackTicket>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT linear_issue_id, linear_issue_url, title, category, description
             FROM feedback_tickets
             WHERE category = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![category, limit as i64], |row| {
            Ok(FeedbackTicket {
                linear_issue_id: row.get(0)?,
                linear_issue_url: row.get(1)?,
                title: row.get(2)?,
                category: row.get(3)?,
                description: row.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn specs_missing_anytype_link(&self) -> anyhow::Result<Vec<SpecMapping>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT linear_issue_id, anytype_object_id, linear_url, anytype_url, last_synced_at
             FROM spec_mappings WHERE anytype_object_id IS NULL"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SpecMapping {
                linear_issue_id: row.get(0)?,
                anytype_object_id: row.get(1)?,
                linear_url: row.get(2)?,
                anytype_url: row.get(3)?,
                last_synced_at: row.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn specs_missing_linear_id(&self) -> anyhow::Result<Vec<SpecMapping>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT linear_issue_id, anytype_object_id, linear_url, anytype_url, last_synced_at
             FROM spec_mappings WHERE linear_issue_id IS NULL OR linear_issue_id = ''"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SpecMapping {
                linear_issue_id: row.get(0)?,
                anytype_object_id: row.get(1)?,
                linear_url: row.get(2)?,
                anytype_url: row.get(3)?,
                last_synced_at: row.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
