use libsql::{Builder, Connection, Database, params};

pub struct Db {
    db: Database,
    persistent_conn: Option<Connection>,
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

#[derive(Debug, Clone)]
pub struct SessionImport {
    pub session_id: String,
    pub session_title: String,
    pub affine_doc_id: Option<String>,
    pub calendar_name: Option<String>,
    pub event_title: Option<String>,
    pub imported_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GithubLinearMapping {
    pub github_issue_number: i64,
    pub github_repo: String,
    pub linear_issue_id: String,
    pub linear_issue_url: Option<String>,
}

impl Db {
    pub async fn open(turso: &crate::config::TursoConfig) -> anyhow::Result<Self> {
        let db = match &turso.replica_path {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut builder = Builder::new_remote_replica(
                    path,
                    turso.url.clone(),
                    turso.auth_token.clone(),
                );
                builder = builder.read_your_writes(true);
                if let Some(secs) = turso.sync_interval_secs {
                    builder = builder.sync_interval(std::time::Duration::from_secs(secs));
                }
                if let Some(ref key) = turso.encryption_key {
                    builder = builder.encryption_config(libsql::EncryptionConfig::new(
                        libsql::Cipher::Aes256Cbc,
                        bytes::Bytes::copy_from_slice(key.as_bytes()),
                    ));
                }
                builder.build().await?
            }
            None if turso.url.starts_with("file:") => {
                let path = turso.url.strip_prefix("file:").unwrap();
                Builder::new_local(path).build().await?
            }
            None => Builder::new_remote(turso.url.clone(), turso.auth_token.clone())
                .build()
                .await?,
        };
        let instance = Db { db, persistent_conn: None };
        instance.migrate().await?;
        if turso.replica_path.is_some() {
            instance.db.sync().await?;
        }
        Ok(instance)
    }

    fn conn(&self) -> anyhow::Result<Connection> {
        if let Some(ref c) = self.persistent_conn {
            return Ok(c.clone());
        }
        Ok(self.db.connect()?)
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        self.conn()?.execute_batch(
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

            CREATE TABLE IF NOT EXISTS github_linear_mappings (
                github_issue_number INTEGER NOT NULL,
                github_repo TEXT NOT NULL,
                linear_issue_id TEXT NOT NULL,
                linear_issue_url TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (github_issue_number, github_repo)
            );

            CREATE TABLE IF NOT EXISTS session_imports (
                session_id TEXT PRIMARY KEY,
                session_title TEXT NOT NULL,
                affine_doc_id TEXT,
                calendar_name TEXT,
                event_title TEXT,
                imported_at TEXT
            );

            CREATE TABLE IF NOT EXISTS study_plans (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                period TEXT NOT NULL,
                affine_doc_id TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_spec_anytype ON spec_mappings(anytype_object_id);
            CREATE INDEX IF NOT EXISTS idx_contract_anytype ON contract_mappings(anytype_object_id);
            CREATE INDEX IF NOT EXISTS idx_contract_linear ON contract_mappings(linear_issue_id);
            CREATE INDEX IF NOT EXISTS idx_event_log_ext ON event_log(source, external_id);
            CREATE INDEX IF NOT EXISTS idx_feedback_category ON feedback_tickets(category);
            CREATE INDEX IF NOT EXISTS idx_gh_linear_mapping ON github_linear_mappings(linear_issue_id);
            CREATE INDEX IF NOT EXISTS idx_imports_calendar ON session_imports(calendar_name);"
        ).await?;
        Ok(())
    }

    pub async fn upsert_spec(&self, mapping: &SpecMapping) -> anyhow::Result<()> {
        self.conn()?.execute(
            "INSERT INTO spec_mappings (linear_issue_id, anytype_object_id, linear_url, anytype_url, last_synced_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(linear_issue_id) DO UPDATE SET
                anytype_object_id = COALESCE(excluded.anytype_object_id, anytype_object_id),
                linear_url = COALESCE(excluded.linear_url, linear_url),
                anytype_url = COALESCE(excluded.anytype_url, anytype_url),
                last_synced_at = datetime('now')",
            params![
                mapping.linear_issue_id.clone(),
                mapping.anytype_object_id.clone(),
                mapping.linear_url.clone(),
                mapping.anytype_url.clone(),
            ],
        ).await?;
        Ok(())
    }

    pub async fn get_spec_by_linear_id(&self, linear_issue_id: &str) -> anyhow::Result<Option<SpecMapping>> {
        let mut rows = self.conn()?.query(
            "SELECT linear_issue_id, anytype_object_id, linear_url, anytype_url, last_synced_at
             FROM spec_mappings WHERE linear_issue_id = ?1",
            params![linear_issue_id.to_string()],
        ).await?;
        match rows.next().await? {
            Some(row) => Ok(Some(SpecMapping {
                linear_issue_id: row.get(0)?,
                anytype_object_id: row.get(1)?,
                linear_url: row.get(2)?,
                anytype_url: row.get(3)?,
                last_synced_at: row.get(4)?,
            })),
            None => Ok(None),
        }
    }

    pub async fn get_spec_by_anytype_id(&self, anytype_object_id: &str) -> anyhow::Result<Option<SpecMapping>> {
        let mut rows = self.conn()?.query(
            "SELECT linear_issue_id, anytype_object_id, linear_url, anytype_url, last_synced_at
             FROM spec_mappings WHERE anytype_object_id = ?1",
            params![anytype_object_id.to_string()],
        ).await?;
        match rows.next().await? {
            Some(row) => Ok(Some(SpecMapping {
                linear_issue_id: row.get(0)?,
                anytype_object_id: row.get(1)?,
                linear_url: row.get(2)?,
                anytype_url: row.get(3)?,
                last_synced_at: row.get(4)?,
            })),
            None => Ok(None),
        }
    }

    pub async fn upsert_contract(&self, mapping: &ContractMapping) -> anyhow::Result<()> {
        self.conn()?.execute(
            "INSERT INTO contract_mappings (documenso_document_id, anytype_object_id, linear_issue_id, status, last_synced_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(documenso_document_id) DO UPDATE SET
                anytype_object_id = COALESCE(excluded.anytype_object_id, anytype_object_id),
                linear_issue_id = COALESCE(excluded.linear_issue_id, linear_issue_id),
                status = COALESCE(excluded.status, status),
                last_synced_at = datetime('now')",
            params![
                mapping.documenso_document_id.clone(),
                mapping.anytype_object_id.clone(),
                mapping.linear_issue_id.clone(),
                mapping.status.clone(),
            ],
        ).await?;
        Ok(())
    }

    pub async fn get_contract_by_documenso_id(&self, doc_id: &str) -> anyhow::Result<Option<ContractMapping>> {
        let mut rows = self.conn()?.query(
            "SELECT documenso_document_id, anytype_object_id, linear_issue_id, status, last_synced_at
             FROM contract_mappings WHERE documenso_document_id = ?1",
            params![doc_id.to_string()],
        ).await?;
        match rows.next().await? {
            Some(row) => Ok(Some(ContractMapping {
                documenso_document_id: row.get(0)?,
                anytype_object_id: row.get(1)?,
                linear_issue_id: row.get(2)?,
                status: row.get(3)?,
                last_synced_at: row.get(4)?,
            })),
            None => Ok(None),
        }
    }

    pub async fn log_event(&self, source: &str, event_type: &str, external_id: &str, payload: Option<&str>) -> anyhow::Result<()> {
        self.conn()?.execute(
            "INSERT INTO event_log (source, event_type, external_id, payload) VALUES (?1, ?2, ?3, ?4)",
            params![
                source.to_string(),
                event_type.to_string(),
                external_id.to_string(),
                payload.map(|s| s.to_string()),
            ],
        ).await?;
        Ok(())
    }

    pub async fn insert_feedback_ticket(
        &self,
        linear_issue_id: &str,
        linear_issue_url: &str,
        title: &str,
        category: &str,
        description: &str,
    ) -> anyhow::Result<()> {
        self.conn()?.execute(
            "INSERT OR IGNORE INTO feedback_tickets
             (linear_issue_id, linear_issue_url, title, category, description)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                linear_issue_id.to_string(),
                linear_issue_url.to_string(),
                title.to_string(),
                category.to_string(),
                description.to_string(),
            ],
        ).await?;
        Ok(())
    }

    pub async fn get_feedback_by_category(&self, category: &str, limit: usize) -> anyhow::Result<Vec<FeedbackTicket>> {
        let mut rows = self.conn()?.query(
            "SELECT linear_issue_id, linear_issue_url, title, category, description
             FROM feedback_tickets
             WHERE category = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
            params![category.to_string(), limit as i64],
        ).await?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(FeedbackTicket {
                linear_issue_id: row.get(0)?,
                linear_issue_url: row.get(1)?,
                title: row.get(2)?,
                category: row.get(3)?,
                description: row.get(4)?,
            });
        }
        Ok(results)
    }

    pub async fn specs_missing_anytype_link(&self) -> anyhow::Result<Vec<SpecMapping>> {
        let mut rows = self.conn()?.query(
            "SELECT linear_issue_id, anytype_object_id, linear_url, anytype_url, last_synced_at
             FROM spec_mappings WHERE anytype_object_id IS NULL",
            params![],
        ).await?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(SpecMapping {
                linear_issue_id: row.get(0)?,
                anytype_object_id: row.get(1)?,
                linear_url: row.get(2)?,
                anytype_url: row.get(3)?,
                last_synced_at: row.get(4)?,
            });
        }
        Ok(results)
    }

    pub async fn specs_missing_linear_id(&self) -> anyhow::Result<Vec<SpecMapping>> {
        let mut rows = self.conn()?.query(
            "SELECT linear_issue_id, anytype_object_id, linear_url, anytype_url, last_synced_at
             FROM spec_mappings WHERE linear_issue_id IS NULL OR linear_issue_id = ''",
            params![],
        ).await?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(SpecMapping {
                linear_issue_id: row.get(0)?,
                anytype_object_id: row.get(1)?,
                linear_url: row.get(2)?,
                anytype_url: row.get(3)?,
                last_synced_at: row.get(4)?,
            });
        }
        Ok(results)
    }

    pub async fn insert_github_linear_mapping(
        &self,
        github_issue_number: i64,
        github_repo: &str,
        linear_issue_id: &str,
        linear_issue_url: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn()?.execute(
            "INSERT OR IGNORE INTO github_linear_mappings
             (github_issue_number, github_repo, linear_issue_id, linear_issue_url)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                github_issue_number,
                github_repo.to_string(),
                linear_issue_id.to_string(),
                linear_issue_url.map(|s| s.to_string()),
            ],
        ).await?;
        Ok(())
    }

    pub async fn get_linear_issue_for_github(&self, github_issue_number: i64, github_repo: &str) -> anyhow::Result<Option<GithubLinearMapping>> {
        let mut rows = self.conn()?.query(
            "SELECT github_issue_number, github_repo, linear_issue_id, linear_issue_url
             FROM github_linear_mappings
             WHERE github_issue_number = ?1 AND github_repo = ?2",
            params![github_issue_number, github_repo.to_string()],
        ).await?;
        match rows.next().await? {
            Some(row) => Ok(Some(GithubLinearMapping {
                github_issue_number: row.get(0)?,
                github_repo: row.get(1)?,
                linear_issue_id: row.get(2)?,
                linear_issue_url: row.get(3)?,
            })),
            None => Ok(None),
        }
    }

    #[cfg(test)]
    pub async fn new_in_memory() -> anyhow::Result<Self> {
        let db = Builder::new_local(":memory:").build().await?;
        let conn = db.connect()?;
        let instance = Db { db, persistent_conn: Some(conn) };
        instance.migrate().await?;
        Ok(instance)
    }

    pub async fn upsert_import(&self, import: &SessionImport) -> anyhow::Result<()> {
        self.conn()?.execute(
            "INSERT INTO session_imports (session_id, session_title, affine_doc_id, calendar_name, event_title, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
             ON CONFLICT(session_id) DO UPDATE SET
                session_title = COALESCE(excluded.session_title, session_title),
                affine_doc_id = COALESCE(excluded.affine_doc_id, affine_doc_id),
                calendar_name = COALESCE(excluded.calendar_name, calendar_name),
                event_title = COALESCE(excluded.event_title, event_title),
                imported_at = datetime('now')",
            params![
                import.session_id.clone(),
                import.session_title.clone(),
                import.affine_doc_id.clone(),
                import.calendar_name.clone(),
                import.event_title.clone(),
            ],
        ).await?;
        Ok(())
    }

    pub async fn get_import(&self, session_id: &str) -> anyhow::Result<Option<SessionImport>> {
        let mut rows = self.conn()?.query(
            "SELECT session_id, session_title, affine_doc_id, calendar_name, event_title, imported_at
             FROM session_imports WHERE session_id = ?1",
            params![session_id.to_string()],
        ).await?;
        match rows.next().await? {
            Some(row) => Ok(Some(SessionImport {
                session_id: row.get(0)?,
                session_title: row.get(1)?,
                affine_doc_id: row.get(2)?,
                calendar_name: row.get(3)?,
                event_title: row.get(4)?,
                imported_at: row.get(5)?,
            })),
            None => Ok(None),
        }
    }

    pub async fn is_imported(&self, session_id: &str) -> anyhow::Result<bool> {
        let mut rows = self.conn()?.query(
            "SELECT 1 FROM session_imports WHERE session_id = ?1",
            params![session_id.to_string()],
        ).await?;
        Ok(rows.next().await?.is_some())
    }

    pub async fn list_imports(&self) -> anyhow::Result<Vec<SessionImport>> {
        let mut rows = self.conn()?.query(
            "SELECT session_id, session_title, affine_doc_id, calendar_name, event_title, imported_at
             FROM session_imports ORDER BY imported_at DESC",
            params![],
        ).await?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(SessionImport {
                session_id: row.get(0)?,
                session_title: row.get(1)?,
                affine_doc_id: row.get(2)?,
                calendar_name: row.get(3)?,
                event_title: row.get(4)?,
                imported_at: row.get(5)?,
            });
        }
        Ok(results)
    }

    pub async fn insert_study_plan(&self, period: &str, affine_doc_id: Option<&str>) -> anyhow::Result<()> {
        self.conn()?.execute(
            "INSERT INTO study_plans (period, affine_doc_id) VALUES (?1, ?2)",
            params![
                period.to_string(),
                affine_doc_id.map(|s| s.to_string()),
            ],
        ).await?;
        Ok(())
    }

    pub async fn get_github_issue_for_linear(
        &self,
        linear_issue_id: &str,
    ) -> anyhow::Result<Option<GithubLinearMapping>> {
        let mut rows = self.conn()?.query(
            "SELECT github_issue_number, github_repo, linear_issue_id, linear_issue_url
             FROM github_linear_mappings
             WHERE linear_issue_id = ?1",
            params![linear_issue_id.to_string()],
        ).await?;
        match rows.next().await? {
            Some(row) => Ok(Some(GithubLinearMapping {
                github_issue_number: row.get(0)?,
                github_repo: row.get(1)?,
                linear_issue_id: row.get(2)?,
                linear_issue_url: row.get(3)?,
            })),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Db {
        Db::new_in_memory().await.expect("in-memory db")
    }

    #[tokio::test]
    async fn spec_roundtrip() {
        let db = db().await;
        let mapping = SpecMapping {
            linear_issue_id: "LIN-1".into(),
            anytype_object_id: Some("AT-1".into()),
            linear_url: Some("https://linear.app/1".into()),
            anytype_url: Some("https://anytype.io/1".into()),
            last_synced_at: None,
        };
        db.upsert_spec(&mapping).await.unwrap();
        let got = db.get_spec_by_linear_id("LIN-1").await.unwrap().unwrap();
        assert_eq!(got.linear_issue_id, "LIN-1");
        assert_eq!(got.anytype_object_id.as_deref(), Some("AT-1"));
    }

    #[tokio::test]
    async fn spec_upsert_preserves_existing_fields() {
        let db = db().await;
        db.upsert_spec(&SpecMapping {
            linear_issue_id: "LIN-2".into(),
            anytype_object_id: Some("AT-2".into()),
            linear_url: Some("https://linear.app/2".into()),
            anytype_url: None,
            last_synced_at: None,
        }).await.unwrap();
        db.upsert_spec(&SpecMapping {
            linear_issue_id: "LIN-2".into(),
            anytype_object_id: None,
            linear_url: None,
            anytype_url: Some("https://anytype.io/2".into()),
            last_synced_at: None,
        }).await.unwrap();
        let got = db.get_spec_by_linear_id("LIN-2").await.unwrap().unwrap();
        assert_eq!(got.anytype_object_id.as_deref(), Some("AT-2"));
        assert_eq!(got.anytype_url.as_deref(), Some("https://anytype.io/2"));
    }

    #[tokio::test]
    async fn spec_by_anytype_id() {
        let db = db().await;
        db.upsert_spec(&SpecMapping {
            linear_issue_id: "LIN-3".into(),
            anytype_object_id: Some("AT-3".into()),
            linear_url: None,
            anytype_url: None,
            last_synced_at: None,
        }).await.unwrap();
        let got = db.get_spec_by_anytype_id("AT-3").await.unwrap().unwrap();
        assert_eq!(got.linear_issue_id, "LIN-3");
    }

    #[tokio::test]
    async fn spec_missing_anytype_link() {
        let db = db().await;
        db.upsert_spec(&SpecMapping {
            linear_issue_id: "LIN-4".into(),
            anytype_object_id: None,
            linear_url: None,
            anytype_url: None,
            last_synced_at: None,
        }).await.unwrap();
        db.upsert_spec(&SpecMapping {
            linear_issue_id: "LIN-5".into(),
            anytype_object_id: Some("AT-5".into()),
            linear_url: None,
            anytype_url: None,
            last_synced_at: None,
        }).await.unwrap();
        let missing = db.specs_missing_anytype_link().await.unwrap();
        assert!(missing.iter().any(|m| m.linear_issue_id == "LIN-4"));
        assert!(!missing.iter().any(|m| m.linear_issue_id == "LIN-5"));
    }

    #[tokio::test]
    async fn contract_roundtrip() {
        let db = db().await;
        let mapping = ContractMapping {
            documenso_document_id: "DOC-1".into(),
            anytype_object_id: Some("AT-C1".into()),
            linear_issue_id: Some("LIN-C1".into()),
            status: Some("pending".into()),
            last_synced_at: None,
        };
        db.upsert_contract(&mapping).await.unwrap();
        let got = db.get_contract_by_documenso_id("DOC-1").await.unwrap().unwrap();
        assert_eq!(got.documenso_document_id, "DOC-1");
        assert_eq!(got.status.as_deref(), Some("pending"));
    }

    #[tokio::test]
    async fn contract_not_found_returns_none() {
        let db = db().await;
        let got = db.get_contract_by_documenso_id("MISSING").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn event_log_insert() {
        let db = db().await;
        db.log_event("github", "issue.opened", "42", Some(r#"{"x":1}"#)).await.unwrap();
        db.log_event("linear", "issue.created", "LIN-99", None).await.unwrap();
    }

    #[tokio::test]
    async fn feedback_ticket_insert_and_query() {
        let db = db().await;
        db.insert_feedback_ticket("LIN-F1", "https://linear.app/f1", "Crash on login", "bug", "App crashes").await.unwrap();
        db.insert_feedback_ticket("LIN-F2", "https://linear.app/f2", "Add dark mode", "feature", "Dark mode request").await.unwrap();

        let bugs = db.get_feedback_by_category("bug", 10).await.unwrap();
        assert_eq!(bugs.len(), 1);
        assert_eq!(bugs[0].title, "Crash on login");

        let features = db.get_feedback_by_category("feature", 10).await.unwrap();
        assert_eq!(features.len(), 1);
    }

    #[tokio::test]
    async fn feedback_ticket_insert_ignore_on_conflict() {
        let db = db().await;
        db.insert_feedback_ticket("LIN-DUP", "https://linear.app/dup", "Original", "bug", "First").await.unwrap();
        db.insert_feedback_ticket("LIN-DUP", "https://linear.app/dup", "Duplicate", "bug", "Second").await.unwrap();

        let bugs = db.get_feedback_by_category("bug", 10).await.unwrap();
        assert_eq!(bugs.len(), 1);
        assert_eq!(bugs[0].title, "Original");
    }

    #[tokio::test]
    async fn github_linear_mapping_roundtrip() {
        let db = db().await;
        db.insert_github_linear_mapping(101, "owner/repo", "LIN-G1", Some("https://linear.app/g1")).await.unwrap();
        let got = db.get_linear_issue_for_github(101, "owner/repo").await.unwrap().unwrap();
        assert_eq!(got.linear_issue_id, "LIN-G1");
        assert_eq!(got.linear_issue_url.as_deref(), Some("https://linear.app/g1"));
    }

    #[tokio::test]
    async fn github_linear_mapping_reverse_lookup() {
        let db = db().await;
        db.insert_github_linear_mapping(202, "owner/repo", "LIN-G2", None).await.unwrap();
        let got = db.get_github_issue_for_linear("LIN-G2").await.unwrap().unwrap();
        assert_eq!(got.github_issue_number, 202);
        assert_eq!(got.github_repo, "owner/repo");
    }

    #[tokio::test]
    async fn github_linear_mapping_not_found() {
        let db = db().await;
        let got = db.get_linear_issue_for_github(999, "no/repo").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn github_linear_mapping_insert_ignore_on_conflict() {
        let db = db().await;
        db.insert_github_linear_mapping(303, "owner/repo", "LIN-ORIG", Some("https://linear.app/orig")).await.unwrap();
        db.insert_github_linear_mapping(303, "owner/repo", "LIN-DUP", None).await.unwrap();
        let got = db.get_linear_issue_for_github(303, "owner/repo").await.unwrap().unwrap();
        assert_eq!(got.linear_issue_id, "LIN-ORIG");
    }

    #[tokio::test]
    async fn session_import_roundtrip() {
        let db = db().await;
        let import = SessionImport {
            session_id: "sess-001".into(),
            session_title: "Lecture Notes".into(),
            affine_doc_id: Some("aff-001".into()),
            calendar_name: Some("Uni".into()),
            event_title: Some("CS101".into()),
            imported_at: None,
        };
        db.upsert_import(&import).await.unwrap();
        let got = db.get_import("sess-001").await.unwrap().unwrap();
        assert_eq!(got.session_id, "sess-001");
        assert_eq!(got.session_title, "Lecture Notes");
        assert_eq!(got.affine_doc_id.as_deref(), Some("aff-001"));
        assert_eq!(got.calendar_name.as_deref(), Some("Uni"));
        assert_eq!(got.event_title.as_deref(), Some("CS101"));
        assert!(got.imported_at.is_some());
    }

    #[tokio::test]
    async fn session_import_upsert_preserves_existing() {
        let db = db().await;
        db.upsert_import(&SessionImport {
            session_id: "sess-002".into(),
            session_title: "Original Title".into(),
            affine_doc_id: Some("aff-002".into()),
            calendar_name: None,
            event_title: Some("Physics".into()),
            imported_at: None,
        }).await.unwrap();
        db.upsert_import(&SessionImport {
            session_id: "sess-002".into(),
            session_title: "Updated Title".into(),
            affine_doc_id: None,
            calendar_name: Some("Uni".into()),
            event_title: None,
            imported_at: None,
        }).await.unwrap();
        let got = db.get_import("sess-002").await.unwrap().unwrap();
        assert_eq!(got.session_title, "Updated Title");
        assert_eq!(got.affine_doc_id.as_deref(), Some("aff-002"));
        assert_eq!(got.calendar_name.as_deref(), Some("Uni"));
        assert_eq!(got.event_title.as_deref(), Some("Physics"));
    }

    #[tokio::test]
    async fn is_imported_true_and_false() {
        let db = db().await;
        db.upsert_import(&SessionImport {
            session_id: "sess-exists".into(),
            session_title: "Exists".into(),
            affine_doc_id: None,
            calendar_name: None,
            event_title: None,
            imported_at: None,
        }).await.unwrap();
        assert!(db.is_imported("sess-exists").await.unwrap());
        assert!(!db.is_imported("sess-missing").await.unwrap());
    }

    #[tokio::test]
    async fn list_imports_returns_all() {
        let db = db().await;
        for i in 0..3 {
            db.upsert_import(&SessionImport {
                session_id: format!("sess-list-{i}"),
                session_title: format!("Session {i}"),
                affine_doc_id: None,
                calendar_name: None,
                event_title: None,
                imported_at: None,
            }).await.unwrap();
        }
        let all = db.list_imports().await.unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.iter().all(|s| s.imported_at.is_some()));
    }

    #[tokio::test]
    async fn insert_study_plan_roundtrip() {
        let db = db().await;
        db.insert_study_plan("2026-W13", Some("aff-sp-001")).await.unwrap();
        db.insert_study_plan("2026-W14", None).await.unwrap();
    }
}
