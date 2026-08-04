//! Durable recording, transcription, and delivery queue.

use crate::paths::{database_path, ensure_dirs, legacy_history_path};
use crate::target::TargetToken;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::PermissionsExt;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid stored JSON data: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub audio_path: String,
    pub transcript: Option<String>,
    pub target: Option<TargetToken>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSummary {
    pub id: String,
    pub sequence: i64,
    pub status: String,
    pub target: Option<String>,
    pub transcript: Option<String>,
    pub created_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub error: Option<String>,
}

pub struct Store {
    connection: Connection,
}

#[derive(Deserialize)]
struct LegacyHistory {
    #[serde(default)]
    entries: Vec<LegacyHistoryEntry>,
}

#[derive(Deserialize)]
struct LegacyHistoryEntry {
    id: String,
    timestamp: DateTime<Utc>,
    text: String,
    audio_path: Option<String>,
    duration_secs: Option<u64>,
}

impl Store {
    pub fn open() -> Result<Self, StoreError> {
        ensure_dirs()?;
        let path = database_path();
        let mut connection = Connection::open(&path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "cache_size", -2000i64)?;
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS jobs (
                id              TEXT PRIMARY KEY,
                sequence        INTEGER NOT NULL UNIQUE,
                status          TEXT NOT NULL,
                audio_path      TEXT NOT NULL,
                transcript      TEXT,
                target_key      TEXT,
                target_json     TEXT,
                created_at_ms   INTEGER NOT NULL,
                ready_at_ms     INTEGER,
                expires_at_ms   INTEGER,
                duration_ms     INTEGER,
                error           TEXT,
                retry_count     INTEGER NOT NULL DEFAULT 0,
                owned_audio     INTEGER NOT NULL DEFAULT 1
            );
            CREATE INDEX IF NOT EXISTS jobs_status_sequence
                ON jobs(status, sequence);
            CREATE INDEX IF NOT EXISTS jobs_target_status_sequence
                ON jobs(target_key, status, sequence);
            CREATE TABLE IF NOT EXISTS metadata (
                key             TEXT PRIMARY KEY,
                value           TEXT NOT NULL
            );
            ",
        )?;
        let has_owned_audio = {
            let mut statement = connection.prepare("PRAGMA table_info(jobs)")?;
            let names = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            names.iter().any(|name| name == "owned_audio")
        };
        if !has_owned_audio {
            connection.execute(
                "ALTER TABLE jobs ADD COLUMN owned_audio INTEGER NOT NULL DEFAULT 1",
                [],
            )?;
        }
        migrate_legacy_history(&mut connection)?;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        Ok(Self { connection })
    }

    pub fn recover(&mut self) -> Result<(), StoreError> {
        self.connection.execute(
            "UPDATE jobs SET status = 'interrupted', error = 'recording interrupted by daemon shutdown'
             WHERE status = 'recording'",
            [],
        )?;
        self.connection.execute(
            "UPDATE jobs SET status = 'queued', error = NULL WHERE status = 'transcribing'",
            [],
        )?;
        self.expire_ready()?;
        Ok(())
    }

    pub fn create_recording(
        &mut self,
        id: &str,
        audio_path: &str,
        target: Option<&TargetToken>,
        owned_audio: bool,
    ) -> Result<i64, StoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sequence: i64 = tx.query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM jobs",
            [],
            |row| row.get(0),
        )?;
        let target_json = target.map(serde_json::to_string).transpose()?;
        let target_key = target.map(TargetToken::key);
        tx.execute(
            "INSERT INTO jobs
             (id, sequence, status, audio_path, target_key, target_json, created_at_ms, owned_audio)
             VALUES (?1, ?2, 'recording', ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                sequence,
                audio_path,
                target_key,
                target_json,
                now_ms(),
                owned_audio
            ],
        )?;
        tx.commit()?;
        Ok(sequence)
    }

    pub fn mark_queued(&mut self, id: &str, duration_ms: i64) -> Result<(), StoreError> {
        self.connection.execute(
            "UPDATE jobs SET status = 'queued', duration_ms = ?2, error = NULL WHERE id = ?1",
            params![id, duration_ms],
        )?;
        Ok(())
    }

    pub fn mark_cancelled(&mut self, id: &str) -> Result<(), StoreError> {
        self.connection.execute(
            "UPDATE jobs SET status = 'cancelled', error = NULL WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn mark_transcribing(&mut self, id: &str) -> Result<bool, StoreError> {
        self.connection
            .execute(
                "UPDATE jobs SET status = 'transcribing', error = NULL
                 WHERE id = ?1 AND status = 'queued'",
                params![id],
            )
            .map(|changed| changed > 0)
            .map_err(StoreError::from)
    }

    pub fn mark_ready(
        &mut self,
        id: &str,
        transcript: &str,
        pending_hours: f64,
    ) -> Result<bool, StoreError> {
        let ready = now_ms();
        let ttl_ms = (pending_hours * 3_600_000.0).round() as i64;
        self.connection
            .execute(
                "UPDATE jobs
             SET status = 'ready', transcript = ?2, ready_at_ms = ?3,
                 expires_at_ms = ?4, error = NULL
             WHERE id = ?1 AND status = 'transcribing'",
                params![id, transcript, ready, ready.saturating_add(ttl_ms)],
            )
            .map(|changed| changed > 0)
            .map_err(StoreError::from)
    }

    pub fn mark_failed(&mut self, id: &str, error: &str) -> Result<(), StoreError> {
        self.connection.execute(
            "UPDATE jobs SET status = 'failed', error = ?2
             WHERE id = ?1 AND status NOT IN ('cancelled', 'delivered')",
            params![id, error],
        )?;
        Ok(())
    }

    pub fn retry(&mut self, id: &str) -> Result<bool, StoreError> {
        let changed = self.connection.execute(
            "UPDATE jobs SET status = 'queued', error = NULL, retry_count = retry_count + 1
             WHERE id = ?1 AND status IN ('failed', 'interrupted')",
            params![id],
        )?;
        Ok(changed > 0)
    }

    pub fn cancel_pending(&mut self, id: &str) -> Result<bool, StoreError> {
        let changed = self.connection.execute(
            "UPDATE jobs SET status = 'cancelled' WHERE id = ?1
             AND status IN ('queued', 'transcribing', 'ready', 'expired', 'failed', 'interrupted')",
            params![id],
        )?;
        Ok(changed > 0)
    }

    pub fn queued(&self) -> Result<Vec<Job>, StoreError> {
        self.load_jobs(
            "SELECT id, audio_path, transcript, target_json
             FROM jobs WHERE status = 'queued' ORDER BY sequence",
            [],
        )
    }

    pub fn get(&self, id: &str) -> Result<Option<Job>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id, audio_path, transcript, target_json
             FROM jobs WHERE id = ?1",
        )?;
        statement
            .query_row(params![id], map_job)
            .optional()
            .map_err(StoreError::from)
    }

    pub fn ready_for_target(&self, target: &TargetToken) -> Result<Vec<Job>, StoreError> {
        self.load_jobs(
            "SELECT id, audio_path, transcript, target_json
             FROM jobs AS candidate
             WHERE candidate.status = 'ready' AND candidate.target_key = ?1
               AND candidate.expires_at_ms > ?2
               AND NOT EXISTS (
                   SELECT 1 FROM jobs AS earlier
                   WHERE earlier.target_key = candidate.target_key
                     AND earlier.sequence < candidate.sequence
                     AND earlier.status IN ('recording', 'queued', 'transcribing')
               )
             ORDER BY sequence",
            params![target.key(), now_ms()],
        )
    }

    pub fn oldest_ready(&self) -> Result<Option<Job>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id, audio_path, transcript, target_json
             FROM jobs
             WHERE (status = 'ready' AND expires_at_ms > ?1) OR status = 'expired'
             ORDER BY sequence LIMIT 1",
        )?;
        statement
            .query_row(params![now_ms()], map_job)
            .optional()
            .map_err(StoreError::from)
    }

    pub fn mark_delivered(&mut self, ids: &[String]) -> Result<(), StoreError> {
        let tx = self.connection.transaction()?;
        for id in ids {
            tx.execute(
                "UPDATE jobs SET status = 'delivered'
                 WHERE id = ?1 AND status IN ('ready', 'expired')",
                params![id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn expire_ready(&mut self) -> Result<usize, StoreError> {
        self.connection
            .execute(
                "UPDATE jobs SET status = 'expired'
                 WHERE status = 'ready' AND expires_at_ms <= ?1",
                params![now_ms()],
            )
            .map_err(StoreError::from)
    }

    pub fn summaries(
        &self,
        pending_only: bool,
        limit: usize,
    ) -> Result<Vec<JobSummary>, StoreError> {
        let where_clause = if pending_only {
            "WHERE status IN ('queued', 'transcribing', 'ready', 'expired', 'failed', 'interrupted')"
        } else {
            "WHERE transcript IS NOT NULL"
        };
        let sql = format!(
            "SELECT id, sequence, status, transcript, target_json, created_at_ms, expires_at_ms, error
             FROM jobs {where_clause} ORDER BY sequence DESC LIMIT ?1"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params![limit as i64], |row| {
            let raw_target: Option<String> = row.get(4)?;
            let target = raw_target
                .and_then(|raw| serde_json::from_str::<TargetToken>(&raw).ok())
                .map(|target| target.label());
            Ok(JobSummary {
                id: row.get(0)?,
                sequence: row.get(1)?,
                status: row.get(2)?,
                transcript: row.get(3)?,
                target,
                created_at_ms: row.get(5)?,
                expires_at_ms: row.get(6)?,
                error: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn counts(&self) -> Result<Vec<(String, usize)>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT status, COUNT(*) FROM jobs GROUP BY status ORDER BY status")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn clear_history(&mut self) -> Result<Vec<String>, StoreError> {
        let tx = self.connection.transaction()?;
        let paths = {
            let mut statement = tx.prepare(
                "SELECT audio_path FROM jobs
                 WHERE owned_audio = 1 AND status NOT IN ('recording', 'queued', 'transcribing')",
            )?;
            let paths = statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            paths
        };
        tx.execute(
            "DELETE FROM jobs WHERE status NOT IN ('recording', 'queued', 'transcribing')",
            [],
        )?;
        tx.commit()?;
        Ok(paths)
    }

    pub fn prune_history(&mut self, maximum: usize) -> Result<Vec<String>, StoreError> {
        if maximum == 0 {
            return Ok(Vec::new());
        }
        let tx = self.connection.transaction()?;
        let rows = {
            let mut statement = tx.prepare(
                "SELECT id, audio_path, owned_audio FROM jobs
                 WHERE transcript IS NOT NULL AND status IN ('delivered', 'expired')
                 ORDER BY sequence DESC LIMIT -1 OFFSET ?1",
            )?;
            let rows = statement
                .query_map(params![maximum as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let mut paths = Vec::new();
        for (id, path, owned) in rows {
            tx.execute("DELETE FROM jobs WHERE id = ?1", params![id])?;
            if owned {
                paths.push(path);
            }
        }
        tx.commit()?;
        Ok(paths)
    }

    fn load_jobs<P>(&self, sql: &str, params: P) -> Result<Vec<Job>, StoreError>
    where
        P: rusqlite::Params,
    {
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(params, map_job)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

fn migrate_legacy_history(connection: &mut Connection) -> Result<(), StoreError> {
    let already_migrated = connection
        .query_row(
            "SELECT 1 FROM metadata WHERE key = 'legacy_history_migrated'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if already_migrated {
        return Ok(());
    }

    let path = legacy_history_path();
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(path)?;
    let history = if raw.trim().is_empty() {
        LegacyHistory {
            entries: Vec::new(),
        }
    } else {
        serde_json::from_str::<LegacyHistory>(&raw)?
    };

    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (index, entry) in history.entries.into_iter().enumerate() {
        let sequence = -i64::try_from(index + 1).unwrap_or(i64::MAX);
        let duration_ms = entry
            .duration_secs
            .map(|seconds| i64::try_from(seconds.saturating_mul(1000)).unwrap_or(i64::MAX));
        tx.execute(
            "INSERT OR IGNORE INTO jobs
             (id, sequence, status, audio_path, transcript, created_at_ms, ready_at_ms,
              duration_ms, owned_audio)
             VALUES (?1, ?2, 'delivered', ?3, ?4, ?5, ?5, ?6, 0)",
            params![
                format!("legacy-{}-{index}", entry.id),
                sequence,
                entry.audio_path.unwrap_or_default(),
                entry.text,
                entry.timestamp.timestamp_millis(),
                duration_ms,
            ],
        )?;
    }
    tx.execute(
        "INSERT INTO metadata(key, value) VALUES ('legacy_history_migrated', '1')",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    let raw_target: Option<String> = row.get(3)?;
    let target = raw_target.and_then(|raw| serde_json::from_str(&raw).ok());
    Ok(Job {
        id: row.get(0)?,
        audio_path: row.get(1)?,
        transcript: row.get(2)?,
        target,
    })
}

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env;

    #[test]
    fn queue_round_trip_and_ordered_delivery() {
        let _guard = test_env::lock();
        let dir = std::env::temp_dir().join(format!("sage-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("SAGE_DATA_DIR", &dir);
        std::env::set_var("SAGE_CONFIG_DIR", dir.join("config"));
        let mut store = Store::open().unwrap();
        let target = TargetToken {
            bus_name: ":1.2".into(),
            object_path: "/field".into(),
            application_name: "Editor".into(),
            window_name: "Notes".into(),
            component_name: "Body".into(),
            role: "Entry".into(),
            editable: true,
        };
        store
            .create_recording("a", "/a.wav", Some(&target), true)
            .unwrap();
        store.mark_queued("a", 400).unwrap();
        store.mark_transcribing("a").unwrap();
        store.mark_ready("a", "first", 2.0).unwrap();
        store
            .create_recording("b", "/b.wav", Some(&target), true)
            .unwrap();
        store.mark_queued("b", 300).unwrap();
        store.mark_transcribing("b").unwrap();
        store.mark_ready("b", "second", 2.0).unwrap();
        let ready = store.ready_for_target(&target).unwrap();
        assert_eq!(
            ready.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        store
            .mark_delivered(&["a".to_string(), "b".to_string()])
            .unwrap();
        assert_eq!(store.prune_history(1).unwrap(), ["/a.wav"]);
        std::env::remove_var("SAGE_DATA_DIR");
        std::env::remove_var("SAGE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancelled_transcription_cannot_become_ready() {
        let _guard = test_env::lock();
        let dir = std::env::temp_dir().join(format!("sage-store-cancel-{}", uuid::Uuid::new_v4()));
        std::env::set_var("SAGE_DATA_DIR", &dir);
        std::env::set_var("SAGE_CONFIG_DIR", dir.join("config"));
        let mut store = Store::open().unwrap();

        store.create_recording("a", "/a.wav", None, true).unwrap();
        store.mark_queued("a", 100).unwrap();
        assert!(store.mark_transcribing("a").unwrap());
        assert!(store.cancel_pending("a").unwrap());
        assert!(!store.mark_ready("a", "late result", 1.0).unwrap());

        std::env::remove_var("SAGE_DATA_DIR");
        std::env::remove_var("SAGE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn imports_legacy_history_once_in_original_order() {
        let _guard = test_env::lock();
        let dir = std::env::temp_dir().join(format!("sage-store-legacy-{}", uuid::Uuid::new_v4()));
        std::env::set_var("SAGE_DATA_DIR", &dir);
        std::env::set_var("SAGE_CONFIG_DIR", dir.join("config"));
        std::fs::create_dir_all(&dir).unwrap();
        let history = serde_json::json!({
            "entries": [
                {
                    "id": "newer",
                    "timestamp": "2026-01-02T00:00:00Z",
                    "text": "newer text",
                    "audio_path": "/legacy/newer.wav",
                    "duration_secs": 2
                },
                {
                    "id": "older",
                    "timestamp": "2026-01-01T00:00:00Z",
                    "text": "older text",
                    "audio_path": null,
                    "duration_secs": null
                }
            ]
        });
        std::fs::write(legacy_history_path(), serde_json::to_vec(&history).unwrap()).unwrap();

        let store = Store::open().unwrap();
        let summaries = store.summaries(false, 10).unwrap();
        assert_eq!(
            summaries
                .iter()
                .map(|entry| entry.transcript.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["newer text", "older text"]
        );
        drop(store);

        std::fs::write(
            legacy_history_path(),
            br#"{"entries":[{"id":"late","timestamp":"2026-01-03T00:00:00Z","text":"must not import","audio_path":null,"duration_secs":null}]}"#,
        )
        .unwrap();
        assert_eq!(
            Store::open().unwrap().summaries(false, 10).unwrap().len(),
            2
        );

        std::env::remove_var("SAGE_DATA_DIR");
        std::env::remove_var("SAGE_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
