use std::sync::Arc;

enum DbInner {
    Local(turso::Database),
    Synced(turso::sync::Database),
}

pub struct Db {
    db: Arc<DbInner>,
}

#[derive(Debug, Clone)]
pub struct SpecMapping {
    pub linear_issue_id: String,
    pub linear_url: Option<String>,
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
    pub linear_issue_id: Option<String>,
    pub status: Option<String>,
    pub last_synced_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GithubLinearMapping {
    pub github_issue_number: i64,
    pub github_repo: String,
    pub linear_issue_id: String,
    pub linear_issue_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TrendingPost {
    pub ticker: String,
    pub posted_at: i64,
    pub post_type: String,
    pub announcement_id: Option<String>,
    pub stonkwatch_link: String,
}

fn text(row: &turso::Row, idx: usize) -> String {
    row.get_value(idx)
        .ok()
        .and_then(|v| v.as_text().map(|s| s.to_string()))
        .unwrap_or_default()
}

fn opt_text(row: &turso::Row, idx: usize) -> Option<String> {
    row.get_value(idx)
        .ok()
        .and_then(|v| v.as_text().map(|s| s.to_string()))
}

fn int(row: &turso::Row, idx: usize) -> i64 {
    row.get_value(idx)
        .ok()
        .and_then(|v| v.as_integer().copied())
        .unwrap_or(0)
}

impl Db {
    pub async fn open(turso_cfg: &crate::config::TursoConfig) -> anyhow::Result<Self> {
        let inner = match &turso_cfg.replica_path {
            Some(path) if !turso_cfg.url.is_empty() => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let path_str = path.to_str().ok_or_else(|| anyhow::anyhow!("invalid UTF-8 in replica path"))?;
                let db = turso::sync::Builder::new_remote(path_str)
                    .with_remote_url(&turso_cfg.url)
                    .with_auth_token(&turso_cfg.auth_token)
                    .build()
                    .await?;
                DbInner::Synced(db)
            }
            Some(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let path_str = path.to_str().ok_or_else(|| anyhow::anyhow!("invalid UTF-8 in replica path"))?;
                let db = turso::Builder::new_local(path_str).build().await?;
                DbInner::Local(db)
            }
            None if turso_cfg.url.starts_with("file:") => {
                let path = turso_cfg.url.strip_prefix("file:").unwrap();
                let db = turso::Builder::new_local(path).build().await?;
                DbInner::Local(db)
            }
            None => {
                let db = turso::sync::Builder::new_remote("/tmp/gluebox-remote.db")
                    .with_remote_url(&turso_cfg.url)
                    .with_auth_token(&turso_cfg.auth_token)
                    .build()
                    .await?;
                DbInner::Synced(db)
            }
        };

        let instance = Db { db: Arc::new(inner) };
        instance.migrate().await?;
        Ok(instance)
    }

    async fn conn(&self) -> anyhow::Result<turso::Connection> {
        match &*self.db {
            DbInner::Local(db) => db.connect().map_err(|e| anyhow::anyhow!("{}", e)),
            DbInner::Synced(db) => db.connect().await.map_err(|e| anyhow::anyhow!("{}", e)),
        }
    }

    async fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn().await?;
        let statements = [
            "CREATE TABLE IF NOT EXISTS spec_mappings (
                linear_issue_id TEXT PRIMARY KEY,
                linear_url TEXT,
                last_synced_at TEXT
            )",
            "CREATE TABLE IF NOT EXISTS contract_mappings (
                documenso_document_id TEXT PRIMARY KEY,
                linear_issue_id TEXT,
                status TEXT,
                last_synced_at TEXT
            )",
            "CREATE TABLE IF NOT EXISTS event_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                event_type TEXT NOT NULL,
                external_id TEXT NOT NULL,
                payload TEXT,
                processed_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
            "CREATE TABLE IF NOT EXISTS feedback_tickets (
                linear_issue_id TEXT PRIMARY KEY,
                linear_issue_url TEXT NOT NULL,
                title TEXT NOT NULL,
                category TEXT NOT NULL,
                description TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
            "CREATE TABLE IF NOT EXISTS github_linear_mappings (
                github_issue_number INTEGER NOT NULL,
                github_repo TEXT NOT NULL,
                linear_issue_id TEXT NOT NULL,
                linear_issue_url TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (github_issue_number, github_repo)
            )",
            "CREATE TABLE IF NOT EXISTS trending_posts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ticker TEXT NOT NULL,
                posted_at INTEGER NOT NULL,
                post_type TEXT NOT NULL,
                announcement_id TEXT,
                stonkwatch_link TEXT NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS idx_trending_posts_ticker_time ON trending_posts(ticker, posted_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_trending_posts_time ON trending_posts(posted_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_contract_linear ON contract_mappings(linear_issue_id)",
            "CREATE INDEX IF NOT EXISTS idx_event_log_ext ON event_log(source, external_id)",
            "CREATE INDEX IF NOT EXISTS idx_feedback_category ON feedback_tickets(category)",
            "CREATE INDEX IF NOT EXISTS idx_gh_linear_mapping ON github_linear_mappings(linear_issue_id)",
        ];
        for stmt in statements {
            conn.execute(stmt, ()).await.map_err(|e| anyhow::anyhow!("migration failed: {}", e))?;
        }
        Ok(())
    }

    pub async fn upsert_spec(&self, mapping: &SpecMapping) -> anyhow::Result<()> {
        self.conn().await?.execute(
            "INSERT INTO spec_mappings (linear_issue_id, linear_url, last_synced_at)
             VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(linear_issue_id) DO UPDATE SET
                linear_url = COALESCE(excluded.linear_url, linear_url),
                last_synced_at = datetime('now')",
            (
                mapping.linear_issue_id.as_str(),
                mapping.linear_url.as_deref().unwrap_or(""),
            ),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    pub async fn get_spec_by_linear_id(&self, linear_issue_id: &str) -> anyhow::Result<Option<SpecMapping>> {
        let conn = self.conn().await?;
        let mut rows = conn.query(
            "SELECT linear_issue_id, linear_url, last_synced_at
             FROM spec_mappings WHERE linear_issue_id = ?1",
            (linear_issue_id,),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        match rows.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
            Some(row) => Ok(Some(SpecMapping {
                linear_issue_id: text(&row, 0),
                linear_url: opt_text(&row, 1),
                last_synced_at: opt_text(&row, 2),
            })),
            None => Ok(None),
        }
    }

    pub async fn upsert_contract(&self, mapping: &ContractMapping) -> anyhow::Result<()> {
        self.conn().await?.execute(
            "INSERT INTO contract_mappings (documenso_document_id, linear_issue_id, status, last_synced_at)
             VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(documenso_document_id) DO UPDATE SET
                linear_issue_id = COALESCE(excluded.linear_issue_id, linear_issue_id),
                status = COALESCE(excluded.status, status),
                last_synced_at = datetime('now')",
            (
                mapping.documenso_document_id.as_str(),
                mapping.linear_issue_id.as_deref().unwrap_or(""),
                mapping.status.as_deref().unwrap_or(""),
            ),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    pub async fn get_contract_by_documenso_id(&self, doc_id: &str) -> anyhow::Result<Option<ContractMapping>> {
        let conn = self.conn().await?;
        let mut rows = conn.query(
            "SELECT documenso_document_id, linear_issue_id, status, last_synced_at
             FROM contract_mappings WHERE documenso_document_id = ?1",
            (doc_id,),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        match rows.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
            Some(row) => Ok(Some(ContractMapping {
                documenso_document_id: text(&row, 0),
                linear_issue_id: opt_text(&row, 1),
                status: opt_text(&row, 2),
                last_synced_at: opt_text(&row, 3),
            })),
            None => Ok(None),
        }
    }

    pub async fn log_event(&self, source: &str, event_type: &str, external_id: &str, payload: Option<&str>) -> anyhow::Result<()> {
        self.conn().await?.execute(
            "INSERT INTO event_log (source, event_type, external_id, payload) VALUES (?1, ?2, ?3, ?4)",
            (source, event_type, external_id, payload.unwrap_or("")),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
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
        self.conn().await?.execute(
            "INSERT OR IGNORE INTO feedback_tickets
             (linear_issue_id, linear_issue_url, title, category, description)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (linear_issue_id, linear_issue_url, title, category, description),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    pub async fn get_feedback_by_category(&self, category: &str, limit: usize) -> anyhow::Result<Vec<FeedbackTicket>> {
        let conn = self.conn().await?;
        let mut rows = conn.query(
            "SELECT linear_issue_id, linear_issue_url, title, category, description
             FROM feedback_tickets
             WHERE category = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
            (category, limit as i64),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
            results.push(FeedbackTicket {
                linear_issue_id: text(&row, 0),
                linear_issue_url: text(&row, 1),
                title: text(&row, 2),
                category: text(&row, 3),
                description: text(&row, 4),
            });
        }
        Ok(results)
    }

    pub async fn specs_missing_linear_id(&self) -> anyhow::Result<Vec<SpecMapping>> {
        let conn = self.conn().await?;
        let mut rows = conn.query(
            "SELECT linear_issue_id, linear_url, last_synced_at
             FROM spec_mappings WHERE linear_issue_id IS NULL OR linear_issue_id = ''",
            (),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
            results.push(SpecMapping {
                linear_issue_id: text(&row, 0),
                linear_url: opt_text(&row, 1),
                last_synced_at: opt_text(&row, 2),
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
        self.conn().await?.execute(
            "INSERT OR IGNORE INTO github_linear_mappings
             (github_issue_number, github_repo, linear_issue_id, linear_issue_url)
             VALUES (?1, ?2, ?3, ?4)",
            (github_issue_number, github_repo, linear_issue_id, linear_issue_url.unwrap_or("")),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    pub async fn get_linear_issue_for_github(&self, github_issue_number: i64, github_repo: &str) -> anyhow::Result<Option<GithubLinearMapping>> {
        let conn = self.conn().await?;
        let mut rows = conn.query(
            "SELECT github_issue_number, github_repo, linear_issue_id, linear_issue_url
             FROM github_linear_mappings
             WHERE github_issue_number = ?1 AND github_repo = ?2",
            (github_issue_number, github_repo),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        match rows.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
            Some(row) => Ok(Some(GithubLinearMapping {
                github_issue_number: int(&row, 0),
                github_repo: text(&row, 1),
                linear_issue_id: text(&row, 2),
                linear_issue_url: opt_text(&row, 3),
            })),
            None => Ok(None),
        }
    }

    pub async fn get_github_issue_for_linear(
        &self,
        linear_issue_id: &str,
    ) -> anyhow::Result<Option<GithubLinearMapping>> {
        let conn = self.conn().await?;
        let mut rows = conn.query(
            "SELECT github_issue_number, github_repo, linear_issue_id, linear_issue_url
             FROM github_linear_mappings
             WHERE linear_issue_id = ?1",
            (linear_issue_id,),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        match rows.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
            Some(row) => Ok(Some(GithubLinearMapping {
                github_issue_number: int(&row, 0),
                github_repo: text(&row, 1),
                linear_issue_id: text(&row, 2),
                linear_issue_url: opt_text(&row, 3),
            })),
            None => Ok(None),
        }
    }

    pub async fn record_trending_post(&self, post: &TrendingPost) -> anyhow::Result<()> {
        self.conn().await?.execute(
            "INSERT INTO trending_posts (ticker, posted_at, post_type, announcement_id, stonkwatch_link)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                post.ticker.as_str(),
                post.posted_at,
                post.post_type.as_str(),
                post.announcement_id.as_deref().unwrap_or(""),
                post.stonkwatch_link.as_str(),
            ),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(())
    }

    pub async fn trending_posts_in_last_24h(&self) -> anyhow::Result<i64> {
        let conn = self.conn().await?;
        let mut rows = conn.query(
            "SELECT COUNT(*) FROM trending_posts WHERE posted_at >= ?1",
            (chrono::Utc::now().timestamp() - 86400,),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        match rows.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
            Some(row) => Ok(int(&row, 0)),
            None => Ok(0),
        }
    }

    pub async fn last_trending_post_for_ticker(&self, ticker: &str) -> anyhow::Result<Option<i64>> {
        let conn = self.conn().await?;
        let mut rows = conn.query(
            "SELECT posted_at FROM trending_posts WHERE ticker = ?1 ORDER BY posted_at DESC LIMIT 1",
            (ticker,),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        match rows.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
            Some(row) => Ok(Some(int(&row, 0))),
            None => Ok(None),
        }
    }

    pub async fn tickers_posted_this_iso_week(&self) -> anyhow::Result<std::collections::HashSet<String>> {
        let monday_ts = current_iso_week_monday_utc_ts()?;
        let conn = self.conn().await?;
        let mut rows = conn.query(
            "SELECT DISTINCT ticker FROM trending_posts WHERE posted_at >= ?1 AND ticker != 'WEEKLY_DIGEST'",
            (monday_ts,),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        let mut set = std::collections::HashSet::new();
        while let Some(row) = rows.next().await.map_err(|e| anyhow::anyhow!("{}", e))? {
            if let Ok(v) = row.get_value(0) {
                if let Some(t) = v.as_text() {
                    set.insert(t.to_string());
                }
            }
        }
        Ok(set)
    }

    pub async fn weekly_digest_posted_this_iso_week(&self) -> anyhow::Result<bool> {
        let monday_ts = current_iso_week_monday_utc_ts()?;
        let conn = self.conn().await?;
        let mut rows = conn.query(
            "SELECT 1 FROM trending_posts WHERE post_type = 'weekly_digest' AND posted_at >= ?1 LIMIT 1",
            (monday_ts,),
        ).await.map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(rows.next().await.map_err(|e| anyhow::anyhow!("{}", e))?.is_some())
    }

    #[cfg(test)]
    pub async fn new_in_memory() -> anyhow::Result<Self> {
        let db = turso::Builder::new_local(":memory:").build().await?;
        let instance = Db { db: Arc::new(DbInner::Local(db)) };
        instance.migrate().await?;
        Ok(instance)
    }
}

fn current_iso_week_monday_utc_ts() -> anyhow::Result<i64> {
    use chrono::Datelike;
    let now = chrono::Utc::now();
    let iso = now.date_naive().iso_week();
    let monday = chrono::NaiveDate::from_isoywd_opt(iso.year(), iso.week(), chrono::Weekday::Mon)
        .ok_or_else(|| anyhow::anyhow!("invalid iso week"))?;
    Ok(monday.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp())
}
