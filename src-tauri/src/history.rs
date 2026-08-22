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
use std::sync::atomic::{AtomicU64, Ordering};
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
/// Re-run the age prune after this many inserts. It otherwise only runs at
/// open, which an app left running for weeks never reaches again.
const PRUNE_EVERY: u64 = 100;

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
    #[error("unsafe history id `{0}`")]
    UnsafeId(String),
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
    /// Counts inserts so [`Self::record`] can re-run the age prune now and
    /// then. The alternative — a background timer — needs a runtime this
    /// deliberately Tauri-free module has no other reason to know about.
    inserts: AtomicU64,
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
            inserts: AtomicU64::new(0),
        };
        store.prune_old()?;
        Ok(store)
    }

    /// [`Self::open`], but a database that cannot be opened — corrupt after a
    /// crash, say — is moved aside and replaced rather than propagated.
    /// History is a convenience; a bad copy of it must not become "the app
    /// panics on every launch until someone finds the file". The broken copy
    /// is kept next to the new one in case any of it is worth recovering.
    pub fn open_or_recover(app_data_dir: &Path) -> Result<Self, HistoryError> {
        match Self::open(app_data_dir) {
            Ok(store) => Ok(store),
            Err(err) => {
                log::warn!("history database unusable ({err}); moving it aside and starting fresh");
                let stamp = now_millis() / 1000;
                // The WAL siblings go with it: a fresh database next to a
                // stale -wal is its own kind of corrupt.
                for name in ["history.db", "history.db-wal", "history.db-shm"] {
                    let path = app_data_dir.join(name);
                    if path.exists() {
                        let _ = fs::rename(&path, app_data_dir.join(format!("{name}.broken-{stamp}")));
                    }
                }
                Self::open(app_data_dir)
            }
        }
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
        // The id becomes a file name when the body spills, and it arrives over
        // the IPC bridge — the same guard section ids get, for the same reason.
        if !crate::store::is_safe_id(&spec.id) {
            return Err(HistoryError::UnsafeId(spec.id.clone()));
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

        // Inbound credentials — Set-Cookie above all — are redacted before
        // they touch disk. The live response pane shows them; a database that
        // outlives the session they belong to should not.
        let headers = response
            .map(|r| serde_json::to_string(&crate::http::redact(&r.headers)))
            .transpose()?;

        // Locks below tolerate poisoning: SQLite's state lives in SQLite, not
        // in whatever a panicked thread was doing with the handle.
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                spec.body.as_deref().unwrap_or_default(),
                error,
                response.map(|r| r.status),
                response.map(|r| r.status_text.as_str()),
                response.map(|r| r.final_url.as_str()),
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
        if self.inserts.fetch_add(1, Ordering::Relaxed) % PRUNE_EVERY == PRUNE_EVERY - 1 {
            self.prune_old()?;
        }
        Ok(())
    }

    /// Metadata for the most recent entries, newest first. Bodies are fetched
    /// separately by [`Self::body`].
    pub fn list(&self, limit: usize) -> Result<Vec<HistoryRecord>, HistoryError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // RETURNING gets the spill paths while deleting in the same indexed
        // statement. The old SELECT-then-DELETE walked the matching rows twice
        // for every per-request prune and every clear.
        let mut statement = connection.prepare(&format!(
            "DELETE FROM history WHERE {predicate} RETURNING body_path"
        ))?;
        let paths: Vec<String> = statement
            .query_map(args.clone(), |row| row.get::<_, Option<String>>(0))?
            .filter_map(Result::ok)
            .flatten()
            .collect();
        drop(statement);
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
            sensitive_header: None,
        ..Default::default()
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
            body_streamed: false,
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

    /// A corrupt database must cost the history, not the app: it is moved
    /// aside — timestamped, in case any of it is recoverable — and a fresh one
    /// takes its place.
    #[test]
    fn a_corrupt_database_is_moved_aside_not_fatal() {
        let dir = std::env::temp_dir().join(format!("fetch-history-broken-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("history.db"), "this is not a sqlite database").unwrap();

        assert!(HistoryStore::open(&dir).is_err(), "plain open should refuse it");

        let store = HistoryStore::open_or_recover(&dir).unwrap();
        store
            .record(&spec("e1", "req-a"), 1000, "https://example.com/x", &Ok(response("{}")))
            .unwrap();
        assert_eq!(store.list(10).unwrap().len(), 1, "the fresh database works");

        let moved_aside = fs::read_dir(&dir).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("history.db.broken-")
        });
        assert!(moved_aside, "the broken file should still be there, renamed");

        let _ = fs::remove_dir_all(dir);
    }

    /// Set-Cookie is a credential the server handed *us*. The pane may show it
    /// while the response is fresh; thirty days of history may not.
    #[test]
    fn inbound_credentials_never_reach_the_database() {
        let (store, dir) = store("redact");
        let mut ok = response("{}");
        ok.headers.push(crate::http::Header {
            name: "set-cookie".into(),
            value: "session=super-secret".into(),
        });
        store
            .record(&spec("e1", "req-a"), 1000, "https://example.com/x", &Ok(ok))
            .unwrap();

        let listed = store.list(10).unwrap();
        let headers = &listed[0].response.as_ref().unwrap().headers;
        let cookie = headers.iter().find(|h| h.name == "set-cookie").unwrap();
        assert_eq!(cookie.value, "<redacted>", "the name survives, the value must not");
        let content_type = headers.iter().find(|h| h.name == "content-type").unwrap();
        assert_eq!(content_type.value, "application/json", "ordinary headers are untouched");

        let _ = fs::remove_dir_all(dir);
    }

    /// The id becomes a spill file's name, and it comes from the webview.
    #[test]
    fn an_id_that_could_escape_the_bodies_directory_is_rejected() {
        let (store, dir) = store("escape");
        let outcome = store.record(
            &spec("../../outside", "req-a"),
            1000,
            "https://example.com/x",
            &Ok(response("{}")),
        );
        assert!(matches!(outcome, Err(HistoryError::UnsafeId(_))), "{outcome:?}");
        assert!(store.list(10).unwrap().is_empty(), "nothing should be recorded");

        let _ = fs::remove_dir_all(dir);
    }

    /// The age prune used to run only at open — useless to an app that stays
    /// running. It now rides along on inserts.
    #[test]
    fn expired_entries_are_pruned_as_new_ones_arrive() {
        let (store, dir) = store("age");
        // Recorded *after* open's prune, with a timestamp far past retention.
        store
            .record(&spec("ancient", "req-old"), 1000, "https://example.com/x", &Ok(response("{}")))
            .unwrap();
        assert!(store.list(500).unwrap().iter().any(|e| e.id == "ancient"));

        for index in 0..PRUNE_EVERY {
            // Distinct request ids, or the per-request cap would be what
            // removes the older ones and the test would prove nothing.
            store
                .record(
                    &spec(&format!("new{index}"), &format!("req-{index}")),
                    now_millis(),
                    "https://example.com/x",
                    &Ok(response("{}")),
                )
                .unwrap();
        }

        let listed = store.list(500).unwrap();
        assert!(!listed.iter().any(|e| e.id == "ancient"), "the prune should have caught it");
        assert!(listed.iter().any(|e| e.id == "new0"), "recent entries stay");

        let _ = fs::remove_dir_all(dir);
    }
}
