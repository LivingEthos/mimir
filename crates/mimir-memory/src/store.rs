//! SQLite-backed memory store with FTS5 full-text search.

use std::path::Path;

use camino::Utf8PathBuf;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Row};
use tracing::{debug, info};

use mimir_schemas::{MemoryEntry, SourceEvidence};

use crate::MemoryError;

/// SQLite-backed memory store.
///
/// Tables:
/// - `entries`: durable memory entries with metadata
/// - `strategies`: strategy knowledge-base entries
/// - `audit_log`: confidence transition audit trail
/// - `entries_fts`: FTS5 virtual table over paths, symbols, packet IDs, previews
#[derive(Debug)]
pub struct MemoryStore {
    conn: Connection,
    db_path: Utf8PathBuf,
}

impl MemoryStore {
    /// Open or create a store at the given database path.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` if SQLite operations fail.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, MemoryError> {
        let path = path.as_ref();
        let conn = Connection::open(path)?;
        let store = Self {
            conn,
            db_path: Utf8PathBuf::from_path_buf(path.to_path_buf())
                .map_err(|p| MemoryError::InvalidPath(p.to_string_lossy().to_string()))?,
        };
        store.init_schema()?;
        info!(db_path = %store.db_path, "MemoryStore opened");
        Ok(store)
    }

    /// Open an in-memory store (useful for tests).
    #[must_use]
    pub fn open_in_memory() -> Self {
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        let store = Self {
            conn,
            db_path: Utf8PathBuf::from(":memory:"),
        };
        store.init_schema().expect("schema init");
        store
    }

    fn init_schema(&self) -> Result<(), MemoryError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entries (
                entry_id TEXT PRIMARY KEY,
                schema_version INTEGER NOT NULL DEFAULT 1,
                kind TEXT NOT NULL,
                body TEXT NOT NULL,
                confidence TEXT NOT NULL,
                promotion_score REAL,
                scope TEXT NOT NULL,
                safe_to_send INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                last_verified_at TEXT,
                retrieval_tags TEXT NOT NULL DEFAULT '[]',
                imported_from TEXT
            );

            CREATE TABLE IF NOT EXISTS strategies (
                strategy_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                applicability TEXT NOT NULL,
                success_count INTEGER NOT NULL DEFAULT 0,
                failure_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS audit_log (
                audit_id TEXT PRIMARY KEY,
                entry_id TEXT NOT NULL,
                old_confidence TEXT,
                new_confidence TEXT NOT NULL,
                reason TEXT NOT NULL,
                changed_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_entries_kind ON entries(kind);
            CREATE INDEX IF NOT EXISTS idx_entries_scope ON entries(scope);
            CREATE INDEX IF NOT EXISTS idx_entries_confidence ON entries(confidence);
            CREATE INDEX IF NOT EXISTS idx_audit_entry_id ON audit_log(entry_id);

            CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
                entry_id UNINDEXED,
                body,
                retrieval_tags,
                content='',
                content_rowid='rowid'
            );

            CREATE TRIGGER IF NOT EXISTS entries_fts_insert AFTER INSERT ON entries BEGIN
                INSERT INTO entries_fts(rowid, entry_id, body, retrieval_tags)
                VALUES (new.rowid, new.entry_id, new.body, new.retrieval_tags);
            END;

            CREATE TRIGGER IF NOT EXISTS entries_fts_delete AFTER DELETE ON entries BEGIN
                INSERT INTO entries_fts(entries_fts, rowid, entry_id, body, retrieval_tags)
                VALUES ('delete', old.rowid, old.entry_id, old.body, old.retrieval_tags);
            END;

            CREATE TRIGGER IF NOT EXISTS entries_fts_update AFTER UPDATE ON entries BEGIN
                INSERT INTO entries_fts(entries_fts, rowid, entry_id, body, retrieval_tags)
                VALUES ('delete', old.rowid, old.entry_id, old.body, old.retrieval_tags);
                INSERT INTO entries_fts(rowid, entry_id, body, retrieval_tags)
                VALUES (new.rowid, new.entry_id, new.body, new.retrieval_tags);
            END;

            CREATE TABLE IF NOT EXISTS source_evidence (
                evidence_id INTEGER PRIMARY KEY AUTOINCREMENT,
                entry_id TEXT NOT NULL REFERENCES entries(entry_id) ON DELETE CASCADE,
                kind TEXT NOT NULL,
                ref_ TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_evidence_entry ON source_evidence(entry_id);

            CREATE TABLE IF NOT EXISTS promotion_breakdown (
                entry_id TEXT PRIMARY KEY REFERENCES entries(entry_id) ON DELETE CASCADE,
                severity_score REAL NOT NULL,
                recurrence_score REAL NOT NULL,
                success_rate_score REAL NOT NULL,
                task_relevance_score REAL NOT NULL,
                token_savings_score REAL NOT NULL,
                total REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS staleness_policy (
                entry_id TEXT PRIMARY KEY REFERENCES entries(entry_id) ON DELETE CASCADE,
                max_age_days INTEGER,
                auto_revoke_after_failures INTEGER
            );

            CREATE TABLE IF NOT EXISTS imported_from (
                entry_id TEXT PRIMARY KEY REFERENCES entries(entry_id) ON DELETE CASCADE,
                tool TEXT NOT NULL,
                session_id TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS entry_tags (
                entry_id TEXT NOT NULL REFERENCES entries(entry_id) ON DELETE CASCADE,
                tag TEXT NOT NULL,
                PRIMARY KEY (entry_id, tag)
            );

            CREATE INDEX IF NOT EXISTS idx_entry_tags_tag ON entry_tags(tag);

            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            );

            INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (1, datetime('now'));

            CREATE TABLE IF NOT EXISTS store_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )?;
        debug!("Schema initialized");
        Ok(())
    }

    /// Insert or replace a memory entry.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn insert(&self, entry: &MemoryEntry) -> Result<(), MemoryError> {
        let tx = self.conn.unchecked_transaction()?;

        self.conn.execute(
            "INSERT OR REPLACE INTO entries (
                entry_id, schema_version, kind, body, confidence,
                promotion_score, scope, safe_to_send, created_at,
                last_verified_at, retrieval_tags, imported_from
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            (
                &entry.entry_id,
                entry.schema_version,
                &entry.kind,
                &entry.body,
                &entry.confidence,
                entry.promotion_score,
                &entry.scope,
                entry.safe_to_send as i32,
                &entry.created_at,
                entry.last_verified_at.as_deref(),
                serde_json::to_string(&entry.retrieval_tags)?,
                entry
                    .imported_from
                    .as_ref()
                    .map(|i| format!("{}:{}", i.tool, i.session_id)),
            ),
        )?;

        // Source evidence
        self.conn.execute(
            "DELETE FROM source_evidence WHERE entry_id = ?1",
            [&entry.entry_id],
        )?;
        for ev in &entry.source_evidence {
            self.conn.execute(
                "INSERT INTO source_evidence (entry_id, kind, ref_) VALUES (?1, ?2, ?3)",
                (&entry.entry_id, &ev.kind, &ev.ref_),
            )?;
        }

        // Promotion breakdown
        if let Some(pb) = &entry.promotion_breakdown {
            self.conn.execute(
                "INSERT OR REPLACE INTO promotion_breakdown (
                    entry_id, severity_score, recurrence_score, success_rate_score,
                    task_relevance_score, token_savings_score, total
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (
                    &entry.entry_id,
                    pb.severity_score,
                    pb.recurrence_score,
                    pb.success_rate_score,
                    pb.task_relevance_score,
                    pb.token_savings_score,
                    pb.total,
                ),
            )?;
        }

        // Staleness policy
        if let Some(sp) = &entry.staleness_policy {
            self.conn.execute(
                "INSERT OR REPLACE INTO staleness_policy (entry_id, max_age_days, auto_revoke_after_failures)
                VALUES (?1, ?2, ?3)",
                (&entry.entry_id, sp.max_age_days, sp.auto_revoke_after_failures),
            )?;
        }

        // Imported from
        if let Some(im) = &entry.imported_from {
            self.conn.execute(
                "INSERT OR REPLACE INTO imported_from (entry_id, tool, session_id)
                VALUES (?1, ?2, ?3)",
                (&entry.entry_id, &im.tool, &im.session_id),
            )?;
        }

        // Tags
        self.conn.execute(
            "DELETE FROM entry_tags WHERE entry_id = ?1",
            [&entry.entry_id],
        )?;
        for tag in &entry.retrieval_tags {
            self.conn.execute(
                "INSERT OR IGNORE INTO entry_tags (entry_id, tag) VALUES (?1, ?2)",
                (&entry.entry_id, tag),
            )?;
        }

        tx.commit()?;
        debug!(entry_id = %entry.entry_id, "Inserted memory entry");
        Ok(())
    }

    /// Get an entry by ID.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn get(&self, id: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT entry_id, schema_version, kind, body, confidence,
                    promotion_score, scope, safe_to_send, created_at,
                    last_verified_at, retrieval_tags, imported_from
             FROM entries WHERE entry_id = ?1",
        )?;
        let row = stmt.query_row([id], Self::row_to_entry).optional()?;
        row.map(|entry| self.hydrate_entry(entry)).transpose()
    }

    /// List all entries, optionally filtered by kind and scope.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn list(
        &self,
        kind: Option<&str>,
        scope: Option<&str>,
        confidence: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let mut sql = String::from(
            "SELECT entry_id, schema_version, kind, body, confidence,
                    promotion_score, scope, safe_to_send, created_at,
                    last_verified_at, retrieval_tags, imported_from
             FROM entries WHERE 1=1",
        );
        if kind.is_some() {
            sql.push_str(" AND kind = ?");
        }
        if scope.is_some() {
            sql.push_str(" AND scope = ?");
        }
        if confidence.is_some() {
            sql.push_str(" AND confidence = ?");
        }
        sql.push_str(" ORDER BY created_at DESC");

        let mut stmt = self.conn.prepare(&sql)?;
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(k) = kind {
            params.push(Box::new(k.to_string()));
        }
        if let Some(s) = scope {
            params.push(Box::new(s.to_string()));
        }
        if let Some(c) = confidence {
            params.push(Box::new(c.to_string()));
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(param_refs.as_slice(), Self::row_to_entry)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(self.hydrate_entry(row?)?);
        }
        Ok(entries)
    }

    /// Full-text search over entries using FTS5.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn search(&self, query: &str) -> Result<Vec<MemoryEntry>, MemoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT e.entry_id, e.schema_version, e.kind, e.body, e.confidence,
                    e.promotion_score, e.scope, e.safe_to_send, e.created_at,
                    e.last_verified_at, e.retrieval_tags, e.imported_from
             FROM entries e
             JOIN entries_fts f ON e.rowid = f.rowid
             WHERE entries_fts MATCH ?1
             ORDER BY rank",
        )?;
        let rows = stmt.query_map([query], Self::row_to_entry)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(self.hydrate_entry(row?)?);
        }
        Ok(entries)
    }

    /// Delete an entry by ID.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn delete(&self, id: &str) -> Result<bool, MemoryError> {
        let rows = self
            .conn
            .execute("DELETE FROM entries WHERE entry_id = ?1", [id])?;
        Ok(rows > 0)
    }

    /// Update confidence with audit logging.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn update_confidence(
        &self,
        id: &str,
        new_confidence: &str,
        reason: &str,
    ) -> Result<bool, MemoryError> {
        let tx = self.conn.unchecked_transaction()?;

        let old: Option<String> = self
            .conn
            .query_row(
                "SELECT confidence FROM entries WHERE entry_id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;

        let rows = self.conn.execute(
            "UPDATE entries SET confidence = ?1 WHERE entry_id = ?2",
            (new_confidence, id),
        )?;

        if rows > 0 {
            let audit_id = uuid::Uuid::new_v4().to_string();
            self.conn.execute(
                "INSERT INTO audit_log (audit_id, entry_id, old_confidence, new_confidence, reason, changed_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    &audit_id,
                    id,
                    old.as_deref(),
                    new_confidence,
                    reason,
                    Utc::now().to_rfc3339(),
                ),
            )?;
            debug!(entry_id = %id, old = ?old, new = %new_confidence, "Confidence updated");
        }

        tx.commit()?;
        Ok(rows > 0)
    }

    /// Get audit log for an entry.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn audit_log(&self, id: &str) -> Result<Vec<AuditRecord>, MemoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT audit_id, entry_id, old_confidence, new_confidence, reason, changed_at
             FROM audit_log WHERE entry_id = ?1 ORDER BY changed_at DESC",
        )?;
        let rows = stmt.query_map([id], |row| {
            Ok(AuditRecord {
                audit_id: row.get(0)?,
                entry_id: row.get(1)?,
                old_confidence: row.get(2)?,
                new_confidence: row.get(3)?,
                reason: row.get(4)?,
                changed_at: row.get(5)?,
            })
        })?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    /// Insert or replace a strategy in the strategy KB.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn upsert_strategy(&self, strategy: &StrategyRecord) -> Result<(), MemoryError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO strategies (
                strategy_id, name, description, applicability,
                success_count, failure_count, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                &strategy.strategy_id,
                &strategy.name,
                &strategy.description,
                &strategy.applicability,
                strategy.success_count,
                strategy.failure_count,
                &strategy.created_at,
                &strategy.updated_at,
            ),
        )?;
        Ok(())
    }

    /// List strategies, optionally filtered by applicability tag.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn list_strategies(
        &self,
        applicability: Option<&str>,
    ) -> Result<Vec<StrategyRecord>, MemoryError> {
        let sql = if applicability.is_some() {
            "SELECT strategy_id, name, description, applicability, success_count, failure_count, created_at, updated_at
             FROM strategies WHERE applicability = ?1 ORDER BY updated_at DESC"
        } else {
            "SELECT strategy_id, name, description, applicability, success_count, failure_count, created_at, updated_at
             FROM strategies ORDER BY updated_at DESC"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = if let Some(app) = applicability {
            stmt.query_map([app], Self::row_to_strategy)?
        } else {
            stmt.query_map([], Self::row_to_strategy)?
        };
        let mut strategies = Vec::new();
        for row in rows {
            strategies.push(row?);
        }
        Ok(strategies)
    }

    /// Record a strategy outcome.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn record_strategy_outcome(
        &self,
        strategy_id: &str,
        success: bool,
    ) -> Result<bool, MemoryError> {
        let rows = self.conn.execute(
            "UPDATE strategies SET
                success_count = success_count + ?1,
                failure_count = failure_count + ?2,
                updated_at = ?3
             WHERE strategy_id = ?4",
            (
                if success { 1 } else { 0 },
                if success { 0 } else { 1 },
                Utc::now().to_rfc3339(),
                strategy_id,
            ),
        )?;
        Ok(rows > 0)
    }

    /// Return the total count of entries.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn count(&self) -> Result<usize, MemoryError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Return the database path.
    #[must_use]
    pub fn db_path(&self) -> &Utf8PathBuf {
        &self.db_path
    }

    fn row_to_entry(row: &Row<'_>) -> Result<MemoryEntry, rusqlite::Error> {
        let tags_json: String = row.get(10)?;
        let retrieval_tags: Vec<String> = serde_json::from_str(&tags_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let imported_from: Option<String> = row.get(11)?;
        let imported_from = imported_from.and_then(|s| {
            let mut parts = s.splitn(2, ':');
            let tool = parts.next()?.to_string();
            let session_id = parts.next()?.to_string();
            Some(mimir_schemas::ImportedFrom { tool, session_id })
        });

        Ok(MemoryEntry {
            schema_version: row.get(1)?,
            entry_id: row.get(0)?,
            kind: row.get(2)?,
            body: row.get(3)?,
            source_evidence: Vec::new(),
            confidence: row.get(4)?,
            promotion_score: row.get(5)?,
            promotion_breakdown: None,
            scope: row.get(6)?,
            safe_to_send: row.get::<_, i32>(7)? != 0,
            created_at: row.get(8)?,
            last_verified_at: row.get(9)?,
            staleness_policy: None,
            retrieval_tags,
            imported_from,
        })
    }

    fn hydrate_entry(&self, mut entry: MemoryEntry) -> Result<MemoryEntry, MemoryError> {
        entry.source_evidence = self.source_evidence(&entry.entry_id)?;
        Ok(entry)
    }

    fn source_evidence(&self, entry_id: &str) -> Result<Vec<SourceEvidence>, MemoryError> {
        let mut stmt = self.conn.prepare(
            "SELECT kind, ref_
             FROM source_evidence
             WHERE entry_id = ?1
             ORDER BY rowid ASC",
        )?;
        let rows = stmt.query_map([entry_id], |row| {
            Ok(SourceEvidence {
                kind: row.get(0)?,
                ref_: row.get(1)?,
            })
        })?;

        let mut evidence = Vec::new();
        for row in rows {
            evidence.push(row?);
        }
        Ok(evidence)
    }

    fn row_to_strategy(row: &Row<'_>) -> Result<StrategyRecord, rusqlite::Error> {
        Ok(StrategyRecord {
            strategy_id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            applicability: row.get(3)?,
            success_count: row.get(4)?,
            failure_count: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }

    /// Rebuild the FTS5 index (useful after bulk imports).
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn rebuild_fts(&self) -> Result<(), MemoryError> {
        // Contentless FTS5 tables don't support DELETE or rebuild.
        // They are maintained by triggers; this is a no-op.
        info!("FTS5 index maintained by triggers; rebuild is a no-op for contentless tables");
        Ok(())
    }

    /// Vacuum the database.
    ///
    /// # Errors
    /// Returns `MemoryError::Database` on SQLite failure.
    pub fn vacuum(&self) -> Result<(), MemoryError> {
        self.conn.execute("VACUUM", [])?;
        Ok(())
    }
}

/// An audit record for a confidence transition.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    /// Unique audit ID.
    pub audit_id: String,
    /// Related entry ID.
    pub entry_id: String,
    /// Previous confidence level.
    pub old_confidence: Option<String>,
    /// New confidence level.
    pub new_confidence: String,
    /// Human-readable reason.
    pub reason: String,
    /// ISO-8601 timestamp.
    pub changed_at: String,
}

/// A strategy knowledge-base record.
#[derive(Debug, Clone)]
pub struct StrategyRecord {
    /// Unique strategy ID.
    pub strategy_id: String,
    /// Human-readable name.
    pub name: String,
    /// Description of the strategy.
    pub description: String,
    /// Applicability tag (e.g., "rust", "python").
    pub applicability: String,
    /// Number of successful applications.
    pub success_count: u32,
    /// Number of failed applications.
    pub failure_count: u32,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// ISO-8601 last update timestamp.
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use mimir_schemas::{ImportedFrom, PromotionBreakdown, SourceEvidence, StalenessPolicy};

    use super::*;

    fn sample_entry(id: &str) -> MemoryEntry {
        MemoryEntry {
            schema_version: 1,
            entry_id: id.to_string(),
            kind: "experience".to_string(),
            body: "Use Result instead of unwrap in Rust".to_string(),
            source_evidence: vec![SourceEvidence {
                kind: "run".to_string(),
                ref_: "run-123".to_string(),
            }],
            confidence: "provisional".to_string(),
            promotion_score: Some(0.75),
            promotion_breakdown: Some(PromotionBreakdown {
                severity_score: 0.8,
                recurrence_score: 0.7,
                success_rate_score: 0.9,
                task_relevance_score: 0.6,
                token_savings_score: 0.5,
                total: 0.75,
            }),
            scope: "repo_shared".to_string(),
            safe_to_send: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            last_verified_at: None,
            staleness_policy: Some(StalenessPolicy {
                max_age_days: Some(30),
                auto_revoke_after_failures: Some(3),
            }),
            retrieval_tags: vec!["rust".to_string(), "error-handling".to_string()],
            imported_from: Some(ImportedFrom {
                tool: "claude-code".to_string(),
                session_id: "sess-abc".to_string(),
            }),
        }
    }

    #[test]
    fn test_open_in_memory() {
        let store = MemoryStore::open_in_memory();
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn test_insert_and_get() {
        let store = MemoryStore::open_in_memory();
        let entry = sample_entry("entry-1");
        store.insert(&entry).unwrap();
        assert_eq!(store.count().unwrap(), 1);

        let fetched = store.get("entry-1").unwrap().unwrap();
        assert_eq!(fetched.entry_id, "entry-1");
        assert_eq!(fetched.kind, "experience");
        assert_eq!(fetched.confidence, "provisional");
        assert_eq!(fetched.source_evidence.len(), 1);
        assert_eq!(fetched.source_evidence[0].kind, "run");
        assert!(fetched.safe_to_send);
        assert_eq!(fetched.retrieval_tags, vec!["rust", "error-handling"]);
    }

    #[test]
    fn test_list_filtered() {
        let store = MemoryStore::open_in_memory();
        let mut e1 = sample_entry("e1");
        e1.kind = "experience".to_string();
        e1.scope = "repo_shared".to_string();
        let mut e2 = sample_entry("e2");
        e2.kind = "error".to_string();
        e2.scope = "global".to_string();
        store.insert(&e1).unwrap();
        store.insert(&e2).unwrap();

        let experience = store.list(Some("experience"), None, None).unwrap();
        assert_eq!(experience.len(), 1);
        assert_eq!(experience[0].entry_id, "e1");

        let global = store.list(None, Some("global"), None).unwrap();
        assert_eq!(global.len(), 1);
        assert_eq!(global[0].entry_id, "e2");
        assert_eq!(global[0].source_evidence.len(), 1);
    }

    #[test]
    fn test_search() {
        let store = MemoryStore::open_in_memory();
        let mut e1 = sample_entry("e1");
        e1.body = "Use Arc for shared ownership".to_string();
        e1.retrieval_tags = vec!["rust".to_string(), "concurrency".to_string()];
        let mut e2 = sample_entry("e2");
        e2.body = "Prefer match over if let chains".to_string();
        e2.retrieval_tags = vec!["rust".to_string(), "style".to_string()];
        store.insert(&e1).unwrap();
        store.insert(&e2).unwrap();
        store.rebuild_fts().unwrap();

        let results = store.search("Arc").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry_id, "e1");
        assert_eq!(results[0].source_evidence.len(), 1);

        let results = store.search("rust").unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_delete() {
        let store = MemoryStore::open_in_memory();
        let entry = sample_entry("del-me");
        store.insert(&entry).unwrap();
        assert_eq!(store.count().unwrap(), 1);
        assert!(store.delete("del-me").unwrap());
        assert_eq!(store.count().unwrap(), 0);
        assert!(!store.delete("del-me").unwrap());
    }

    #[test]
    fn test_update_confidence_audits() {
        let store = MemoryStore::open_in_memory();
        let entry = sample_entry("audit-test");
        store.insert(&entry).unwrap();

        assert!(store
            .update_confidence("audit-test", "validated", "human review")
            .unwrap());
        let fetched = store.get("audit-test").unwrap().unwrap();
        assert_eq!(fetched.confidence, "validated");

        let log = store.audit_log("audit-test").unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].old_confidence.as_deref(), Some("provisional"));
        assert_eq!(log[0].new_confidence, "validated");
        assert_eq!(log[0].reason, "human review");
    }

    #[test]
    fn test_strategies() {
        let store = MemoryStore::open_in_memory();
        let s = StrategyRecord {
            strategy_id: "s1".to_string(),
            name: "Use anyhow".to_string(),
            description: "Replace manual error types with anyhow".to_string(),
            applicability: "rust".to_string(),
            success_count: 0,
            failure_count: 0,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        store.upsert_strategy(&s).unwrap();
        let list = store.list_strategies(None).unwrap();
        assert_eq!(list.len(), 1);

        store.record_strategy_outcome("s1", true).unwrap();
        let list = store.list_strategies(None).unwrap();
        assert_eq!(list[0].success_count, 1);
        assert_eq!(list[0].failure_count, 0);
    }

    #[test]
    fn test_search_no_results() {
        let store = MemoryStore::open_in_memory();
        let results = store.search("nonexistent").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_vacuum() {
        let store = MemoryStore::open_in_memory();
        store.vacuum().unwrap();
    }
}
