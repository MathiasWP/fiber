//! Request history, in SQLite.
//!
//! Written from Rust as part of sending, not by the frontend afterwards: the
//! response body is already here, so round-tripping it back down just to store
//! it would double the cost of every large response — and history stays honest
//! even if the window goes away mid-request.
//!
//! Bodies above [`SPILL_BYTES`] go to a file and the row keeps a path, so
//! listing history never drags megabytes of JSON into memory.
//!
//! Free of Tauri types, like the rest of the core, so the MCP server can answer
//! "what did that endpoint return last time?" without a window.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::http::{Header, HttpError, RequestSpec, ResponseData, Timing};

/// Bodies larger than this live in a file rather than in the row.
const SPILL_BYTES: usize = 256 * 1024;
/// Per request, newest kept. Enough to compare a few attempts, not a log.
const MAX_PER_REQUEST: usize = 50;
const MAX_AGE_DAYS: i64 = 30;

/// Everything about a response except the body, which is fetched separately.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseMeta {
    pub status: u16,
    pub status_text: String,
    pub final_url: String,
    pub headers: Vec<Header>,
    pub is_binary: bool,
    pub truncated: bool,
    pub size_bytes: u64,
    pub timing: Timing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    pub id: String,
    /// The saved request this belongs to, or `scratch`.
    pub request_id: String,
    /// Epoch milliseconds.
    pub at: i64,
    pub method: String,
    pub url: String,
    pub request_body: String,
    /// Exactly one of these is set.
    pub response: Option<ResponseMeta>,
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("history database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("history body: {0}")]
    Io(#[from] std::io::Error),
    #[error("history data: {0}")]
    Encoding(#[from] serde_json::Error),
}

impl Serialize for HistoryError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

pub struct HistoryStore {
    connection: Mutex<Connection>,
    bodies_dir: PathBuf,
}

impl HistoryStore {
    pub fn open(app_data_dir: &Path) -> Result<Self, HistoryError> {
        fs::create_dir_all(app_data_dir)?;
        let bodies_dir = app_data_dir.join("bodies");
        fs::create_dir_all(&bodies_dir)?;

        let connection = Connection::open(app_data_dir.join("history.db"))?;
        // WAL keeps reads from blocking behind the write that a send performs.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS history (
                id           TEXT PRIMARY KEY,
                request_id   TEXT NOT NULL,
                at           INTEGER NOT NULL,
                method       TEXT NOT NULL,
                url          TEXT NOT NULL,
                request_body TEXT NOT NULL DEFAULT '',
                error        TEXT,
                status       INTEGER,
                status_text  TEXT,
                final_url    TEXT,
                headers      TEXT,
                is_binary    INTEGER NOT NULL DEFAULT 0,
                truncated    INTEGER NOT NULL DEFAULT 0,
                size_bytes   INTEGER,
                ttfb_ms      INTEGER,
                total_ms     INTEGER,
                body         TEXT,
                body_path    TEXT
            );
            CREATE INDEX IF NOT EXISTS history_by_request ON history(request_id, at DESC);
            CREATE INDEX IF NOT EXISTS history_by_time ON history(at DESC);",
        )?;

        let store = Self {
            connection: Mutex::new(connection),
            bodies_dir,
        };
        store.prune_old()?;
        Ok(store)
    }

    fn spill_path(&self, id: &str) -> PathBuf {
        self.bodies_dir.join(format!("{id}.body"))
    }

    /// Records the outcome of a send. Cancelled requests are deliberately not
    /// recorded — the user aborted, there's nothing to look back at.
    pub fn record(
        &self,
        spec: &RequestSpec,
        at: i64,
        url: &str,
        outcome: &Result<ResponseData, HttpError>,
    ) -> Result<(), HistoryError> {
        if matches!(outcome, Err(HttpError::Cancelled)) {
            return Ok(());
        }

        let (error, response) = match outcome {
            Ok(response) => (None, Some(response)),
            Err(err) => (Some(err.to_string()), None),
        };

        // Large bodies go to disk so `list` stays cheap.
        let mut inline_body: Option<&str> = None;
        let mut body_path: Option<String> = None;
        if let Some(response) = response {
            if response.body.len() > SPILL_BYTES {
                let path = self.spill_path(&spec.id);
                fs::write(&path, &response.body)?;
                body_path = Some(path.display().to_string());
            } else {
                inline_body = Some(&response.body);
            }
        }

        let headers = response
            .map(|r| serde_json::to_string(&r.headers))
            .transpose()?;

        let connection = self.connection.lock().unwrap();
        connection.execute(
            "INSERT OR REPLACE INTO history (
                id, request_id, at, method, url, request_body, error,
                status, status_text, final_url, headers, is_binary, truncated,
                size_bytes, ttfb_ms, total_ms, body, body_path
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![
                spec.id,
                spec.request_id,
                at,
                spec.method,
                url,
                spec.body.clone().unwrap_or_default(),
                error,
                response.map(|r| r.status),
                response.map(|r| r.status_text.clone()),
                response.map(|r| r.final_url.clone()),
                headers,
                response.map(|r| r.is_binary).unwrap_or(false),
                response.map(|r| r.truncated).unwrap_or(false),
                // SQLite integers are signed; these are byte counts and
                // millisecond durations, so the range is never a concern.
                response.map(|r| r.size_bytes as i64),
                response.map(|r| r.timing.ttfb_ms as i64),
                response.map(|r| r.timing.total_ms as i64),
                inline_body,
                body_path,
            ],
        )?;
        drop(connection);

        self.prune_request(&spec.request_id)?;
        Ok(())
    }

    /// Metadata for the most recent entries, newest first. Bodies are fetched
    /// separately by [`Self::body`].
    pub fn list(&self, limit: usize) -> Result<Vec<HistoryRecord>, HistoryError> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, request_id, at, method, url, request_body, error,
                    status, status_text, final_url, headers, is_binary, truncated,
                    size_bytes, ttfb_ms, total_ms
             FROM history ORDER BY at DESC LIMIT ?1",
        )?;

        let rows = statement.query_map(params![limit as i64], |row| {
            let status: Option<u16> = row.get(7)?;
            let headers: Option<String> = row.get(10)?;
            let response = status.map(|status| ResponseMeta {
                status,
                status_text: row.get::<_, Option<String>>(8).unwrap_or_default().unwrap_or_default(),
                final_url: row.get::<_, Option<String>>(9).unwrap_or_default().unwrap_or_default(),
                headers: headers
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default(),
                is_binary: row.get(11).unwrap_or(false),
                truncated: row.get(12).unwrap_or(false),
                size_bytes: row.get::<_, Option<i64>>(13).unwrap_or_default().unwrap_or_default()
                    as u64,
                timing: Timing {
                    ttfb_ms: row.get::<_, Option<i64>>(14).unwrap_or_default().unwrap_or_default()
                        as u64,
                    total_ms: row.get::<_, Option<i64>>(15).unwrap_or_default().unwrap_or_default()
                        as u64,
                },
            });

            Ok(HistoryRecord {
                id: row.get(0)?,
                request_id: row.get(1)?,
                at: row.get(2)?,
                method: row.get(3)?,
                url: row.get(4)?,
                request_body: row.get(5)?,
                error: row.get(6)?,
                response,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// The response body for one entry, read from the row or its spill file.
    pub fn body(&self, id: &str) -> Result<Option<String>, HistoryError> {
        let connection = self.connection.lock().unwrap();
        let found: Option<(Option<String>, Option<String>)> = connection
            .query_row(
                "SELECT body, body_path FROM history WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        drop(connection);

        match found {
            Some((Some(body), _)) => Ok(Some(body)),
            // A missing spill file shouldn't error the UI — the metadata is
            // still worth showing.
            Some((None, Some(path))) => Ok(fs::read_to_string(path).ok()),
            _ => Ok(None),
        }
    }

    pub fn delete(&self, id: &str) -> Result<(), HistoryError> {
        self.remove_where("id = ?1", params![id])
    }

    pub fn clear_request(&self, request_id: &str) -> Result<(), HistoryError> {
        self.remove_where("request_id = ?1", params![request_id])
    }

    pub fn clear_all(&self) -> Result<(), HistoryError> {
        self.remove_where("1 = 1", params![])
    }

    fn prune_old(&self) -> Result<(), HistoryError> {
        let cutoff = now_millis() - MAX_AGE_DAYS * 24 * 60 * 60 * 1000;
        self.remove_where("at < ?1", params![cutoff])
    }

    /// Keeps only the newest [`MAX_PER_REQUEST`] entries for one request.
    fn prune_request(&self, request_id: &str) -> Result<(), HistoryError> {
        self.remove_where(
            "request_id = ?1 AND id NOT IN (
                 SELECT id FROM history WHERE request_id = ?1 ORDER BY at DESC LIMIT ?2
             )",
            params![request_id, MAX_PER_REQUEST as i64],
        )
    }

    /// Deletes rows and any spill files they own, so the bodies directory can't
    /// outlive the database.
    fn remove_where(
        &self,
        predicate: &str,
        args: impl rusqlite::Params + Clone,
    ) -> Result<(), HistoryError> {
        let connection = self.connection.lock().unwrap();

        let mut statement =
            connection.prepare(&format!("SELECT body_path FROM history WHERE {predicate}"))?;
        let paths: Vec<String> = statement
            .query_map(args.clone(), |row| row.get::<_, Option<String>>(0))?
            .filter_map(Result::ok)
            .flatten()
            .collect();
        drop(statement);

        connection.execute(&format!("DELETE FROM history WHERE {predicate}"), args)?;
        drop(connection);

        for path in paths {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, request_id: &str) -> RequestSpec {
        RequestSpec {
            id: id.into(),
            request_id: request_id.into(),
            section_id: None,
            method: "GET".into(),
            url: "https://example.com/x".into(),
            headers: vec![],
            body: None,
            timeout_ms: None,
            follow_redirects: true,
            accept_invalid_certs: false,
        }
    }

    fn response(body: &str) -> ResponseData {
        ResponseData {
            status: 200,
            status_text: "OK".into(),
            final_url: "https://example.com/x".into(),
            headers: vec![Header {
                name: "content-type".into(),
                value: "application/json".into(),
            }],
            body: body.to_string(),
            is_binary: false,
            truncated: false,
            size_bytes: body.len() as u64,
            timing: Timing {
                ttfb_ms: 10,
                total_ms: 20,
            },
        }
    }

    fn store(name: &str) -> (HistoryStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("fetch-history-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        (HistoryStore::open(&dir).unwrap(), dir)
    }

    #[test]
    fn round_trips_a_response() {
        let (store, dir) = store("round");
        store
            .record(&spec("e1", "req-a"), 1000, "https://example.com/x", &Ok(response("{\"ok\":true}")))
            .unwrap();

        let listed = store.list(10).unwrap();
        assert_eq!(listed.len(), 1);
        let entry = &listed[0];
        assert_eq!(entry.request_id, "req-a");
        let meta = entry.response.as_ref().unwrap();
        assert_eq!(meta.status, 200);
        assert_eq!(meta.headers[0].name, "content-type");
        assert_eq!(meta.timing.total_ms, 20);
        assert_eq!(store.body("e1").unwrap().unwrap(), "{\"ok\":true}");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn records_failures_but_not_cancellations() {
        let (store, dir) = store("errors");
        store
            .record(
                &spec("e1", "req-a"),
                1000,
                "https://example.com/x",
                &Err(HttpError::Timeout),
            )
            .unwrap();
        store
            .record(
                &spec("e2", "req-a"),
                2000,
                "https://example.com/x",
                &Err(HttpError::Cancelled),
            )
            .unwrap();

        let listed = store.list(10).unwrap();
        assert_eq!(listed.len(), 1, "cancelled requests are not history");
        assert_eq!(listed[0].error.as_deref(), Some("request timed out"));
        assert!(listed[0].response.is_none());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn spills_large_bodies_to_disk_and_reads_them_back() {
        let (store, dir) = store("spill");
        let big = "x".repeat(SPILL_BYTES + 10);
        store
            .record(&spec("e1", "req-a"), 1000, "https://example.com/x", &Ok(response(&big)))
            .unwrap();

        assert!(dir.join("bodies").join("e1.body").exists(), "should have spilled");
        assert_eq!(store.body("e1").unwrap().unwrap().len(), big.len());

        // Deleting the row takes the file with it.
        store.delete("e1").unwrap();
        assert!(!dir.join("bodies").join("e1.body").exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_only_the_newest_entries_per_request() {
        let (store, dir) = store("prune");
        for index in 0..(MAX_PER_REQUEST + 15) {
            store
                .record(
                    &spec(&format!("e{index}"), "req-a"),
                    1000 + index as i64,
                    "https://example.com/x",
                    &Ok(response("{}")),
                )
                .unwrap();
        }
        // A second request must not be affected by the first's pruning.
        store
            .record(&spec("other", "req-b"), 5, "https://example.com/y", &Ok(response("{}")))
            .unwrap();

        let all = store.list(500).unwrap();
        let for_a = all.iter().filter(|e| e.request_id == "req-a").count();
        assert_eq!(for_a, MAX_PER_REQUEST);
        assert!(all.iter().any(|e| e.request_id == "req-b"));
        // The oldest went, the newest stayed.
        assert!(!all.iter().any(|e| e.id == "e0"));
        assert!(all.iter().any(|e| e.id == "e64"));

        let _ = fs::remove_dir_all(dir);
    }

    /// Mirrors what the `send_request` command does: send, then record the
    /// outcome. Guards the seam between the two modules.
    #[tokio::test]
    async fn records_a_real_send() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            socket
                .write_all(
                    b"HTTP/1.1 418 I'm a teapot\r\n\
                      Content-Type: application/json\r\n\
                      Content-Length: 12\r\n\
                      \r\n\
                      {\"tea\":true}",
                )
                .await
                .unwrap();
            socket.flush().await.unwrap();
        });

        let (store, dir) = store("integration");
        let mut spec = spec("live-1", "req-live");
        spec.url = format!("http://{addr}/brew");

        let http_state = crate::http::HttpState::default();
        let at = now_millis();
        let url = spec.url.clone();
        let outcome = crate::http::send(&http_state, spec.clone()).await;
        store.record(&spec, at, &url, &outcome).unwrap();

        let listed = store.list(10).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].request_id, "req-live");
        assert_eq!(listed[0].url, format!("http://{addr}/brew"));
        assert_eq!(listed[0].response.as_ref().unwrap().status, 418);
        assert_eq!(store.body("live-1").unwrap().unwrap(), "{\"tea\":true}");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn clears_by_request_and_wholesale() {
        let (store, dir) = store("clear");
        store.record(&spec("a1", "req-a"), 1, "u", &Ok(response("{}"))).unwrap();
        store.record(&spec("b1", "req-b"), 2, "u", &Ok(response("{}"))).unwrap();

        store.clear_request("req-a").unwrap();
        let left = store.list(10).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].request_id, "req-b");

        store.clear_all().unwrap();
        assert!(store.list(10).unwrap().is_empty());

        let _ = fs::remove_dir_all(dir);
    }
}
