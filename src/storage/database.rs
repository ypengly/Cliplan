use rusqlite::{params, Connection};
use std::path::Path;

use super::models::{ClipboardEntry, Device, PairingRequest, Transfer};

/// Open (creating if necessary) the sqlite database and apply the schema.
/// sqlite is used only for metadata -- actual file bytes always live on
/// disk under the configured shared/files directory (see config.rs).
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS devices (
            id           TEXT PRIMARY KEY,
            name         TEXT NOT NULL,
            fingerprint  TEXT NOT NULL,
            token_hash   TEXT NOT NULL UNIQUE,
            created_at   TEXT NOT NULL,
            last_seen    TEXT NOT NULL,
            status       TEXT NOT NULL DEFAULT 'offline'
        );

        CREATE TABLE IF NOT EXISTS pairing_requests (
            id           TEXT PRIMARY KEY,
            device_name  TEXT NOT NULL,
            fingerprint  TEXT NOT NULL,
            code         TEXT NOT NULL,
            status       TEXT NOT NULL DEFAULT 'pending',
            created_at   TEXT NOT NULL,
            expires_at   TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS clipboard_entries (
            id           TEXT PRIMARY KEY,
            content      TEXT NOT NULL,
            content_type TEXT NOT NULL,
            device_id    TEXT,
            created_at   TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS transfers (
            id              TEXT PRIMARY KEY,
            filename        TEXT NOT NULL,
            size            INTEGER NOT NULL,
            sha256          TEXT,
            from_device     TEXT,
            to_device       TEXT,
            status          TEXT NOT NULL DEFAULT 'pending',
            bytes_received  INTEGER NOT NULL DEFAULT 0,
            storage_path    TEXT NOT NULL,
            created_at      TEXT NOT NULL,
            completed_at    TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_clipboard_created ON clipboard_entries(created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_transfers_created ON transfers(created_at DESC);
        "#,
    )
}

// ---------- Devices ----------

pub fn insert_device(
    conn: &Connection,
    id: &str,
    name: &str,
    fingerprint: &str,
    token_hash: &str,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO devices (id, name, fingerprint, token_hash, created_at, last_seen, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, 'online')",
        params![id, name, fingerprint, token_hash, now],
    )?;
    Ok(())
}

pub fn find_device_by_token_hash(
    conn: &Connection,
    token_hash: &str,
) -> rusqlite::Result<Option<Device>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, fingerprint, created_at, last_seen, status FROM devices WHERE token_hash = ?1",
    )?;
    let mut rows = stmt.query(params![token_hash])?;
    if let Some(row) = rows.next()? {
        Ok(Some(Device {
            id: row.get(0)?,
            name: row.get(1)?,
            fingerprint: row.get(2)?,
            created_at: row.get(3)?,
            last_seen: row.get(4)?,
            status: row.get(5)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn list_devices(conn: &Connection) -> rusqlite::Result<Vec<Device>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, fingerprint, created_at, last_seen, status FROM devices ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Device {
            id: row.get(0)?,
            name: row.get(1)?,
            fingerprint: row.get(2)?,
            created_at: row.get(3)?,
            last_seen: row.get(4)?,
            status: row.get(5)?,
        })
    })?;
    rows.collect()
}

pub fn touch_device(conn: &Connection, id: &str, now: &str, status: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE devices SET last_seen = ?1, status = ?2 WHERE id = ?3",
        params![now, status, id],
    )?;
    Ok(())
}

pub fn rename_device(conn: &Connection, id: &str, name: &str) -> rusqlite::Result<usize> {
    conn.execute("UPDATE devices SET name = ?1 WHERE id = ?2", params![name, id])
}

pub fn delete_device(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM devices WHERE id = ?1", params![id])
}

// ---------- Pairing requests ----------

pub fn insert_pairing_request(
    conn: &Connection,
    id: &str,
    device_name: &str,
    fingerprint: &str,
    code: &str,
    now: &str,
    expires_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO pairing_requests (id, device_name, fingerprint, code, status, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6)",
        params![id, device_name, fingerprint, code, now, expires_at],
    )?;
    Ok(())
}

pub fn get_pairing_request(conn: &Connection, id: &str) -> rusqlite::Result<Option<PairingRequest>> {
    let mut stmt = conn.prepare(
        "SELECT id, device_name, fingerprint, status, created_at, expires_at FROM pairing_requests WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(PairingRequest {
            id: row.get(0)?,
            device_name: row.get(1)?,
            fingerprint: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            expires_at: row.get(5)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn list_pending_pairing_requests(conn: &Connection) -> rusqlite::Result<Vec<PairingRequest>> {
    let mut stmt = conn.prepare(
        "SELECT id, device_name, fingerprint, status, created_at, expires_at
         FROM pairing_requests WHERE status = 'pending' ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(PairingRequest {
            id: row.get(0)?,
            device_name: row.get(1)?,
            fingerprint: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            expires_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

pub fn set_pairing_status(conn: &Connection, id: &str, status: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE pairing_requests SET status = ?1 WHERE id = ?2",
        params![status, id],
    )
}

// ---------- Clipboard ----------

pub fn insert_clipboard_entry(
    conn: &Connection,
    id: &str,
    content: &str,
    content_type: &str,
    device_id: Option<&str>,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO clipboard_entries (id, content, content_type, device_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, content, content_type, device_id, now],
    )?;
    Ok(())
}

pub fn list_clipboard_entries(
    conn: &Connection,
    limit: usize,
) -> rusqlite::Result<Vec<ClipboardEntry>> {
    let mut stmt = conn.prepare(
        "SELECT ce.id, ce.content, ce.content_type, ce.device_id, d.name, ce.created_at
         FROM clipboard_entries ce
         LEFT JOIN devices d ON d.id = ce.device_id
         ORDER BY ce.created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(ClipboardEntry {
            id: row.get(0)?,
            content: row.get(1)?,
            content_type: row.get(2)?,
            device_id: row.get(3)?,
            device_name: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

pub fn delete_clipboard_entry(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM clipboard_entries WHERE id = ?1", params![id])
}

pub fn clear_clipboard_history(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM clipboard_entries", [])
}

/// Trim the clipboard history table down to `max` most-recent rows.
pub fn trim_clipboard_history(conn: &Connection, max: usize) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM clipboard_entries WHERE id NOT IN (
            SELECT id FROM clipboard_entries ORDER BY created_at DESC LIMIT ?1
        )",
        params![max as i64],
    )
}

// ---------- Transfers ----------

#[allow(clippy::too_many_arguments)]
pub fn insert_transfer(
    conn: &Connection,
    id: &str,
    filename: &str,
    size: i64,
    from_device: Option<&str>,
    to_device: Option<&str>,
    storage_path: &str,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO transfers (id, filename, size, from_device, to_device, status, bytes_received, storage_path, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 0, ?6, ?7)",
        params![id, filename, size, from_device, to_device, storage_path, now],
    )?;
    Ok(())
}

pub fn update_transfer_progress(conn: &Connection, id: &str, bytes_received: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE transfers SET bytes_received = ?1, status = 'transferring' WHERE id = ?2",
        params![bytes_received, id],
    )?;
    Ok(())
}

pub fn complete_transfer(
    conn: &Connection,
    id: &str,
    sha256: &str,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE transfers SET status = 'completed', sha256 = ?1, completed_at = ?2 WHERE id = ?3",
        params![sha256, now, id],
    )?;
    Ok(())
}

pub fn fail_transfer(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE transfers SET status = 'failed' WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn get_transfer(conn: &Connection, id: &str) -> rusqlite::Result<Option<Transfer>> {
    let mut stmt = conn.prepare(
        "SELECT id, filename, size, sha256, from_device, to_device, status, bytes_received, created_at, completed_at
         FROM transfers WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_transfer(row)?))
    } else {
        Ok(None)
    }
}

pub fn get_transfer_storage_path(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT storage_path FROM transfers WHERE id = ?1")?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}

pub fn list_transfers(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<Transfer>> {
    let mut stmt = conn.prepare(
        "SELECT id, filename, size, sha256, from_device, to_device, status, bytes_received, created_at, completed_at
         FROM transfers ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], row_to_transfer)?;
    rows.collect()
}

pub fn delete_transfer(conn: &Connection, id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM transfers WHERE id = ?1", params![id])
}

pub fn clear_transfer_history(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM transfers WHERE status IN ('completed', 'failed', 'cancelled')", [])
}

/// Storage paths for every finished transfer (completed/failed/cancelled),
/// used when clearing history so the underlying files are removed too
/// instead of being left orphaned on disk.
pub fn list_clearable_storage_paths(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT storage_path FROM transfers WHERE status IN ('completed', 'failed', 'cancelled')",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

fn row_to_transfer(row: &rusqlite::Row) -> rusqlite::Result<Transfer> {
    Ok(Transfer {
        id: row.get(0)?,
        filename: row.get(1)?,
        size: row.get(2)?,
        sha256: row.get(3)?,
        from_device: row.get(4)?,
        to_device: row.get(5)?,
        status: row.get(6)?,
        bytes_received: row.get(7)?,
        created_at: row.get(8)?,
        completed_at: row.get(9)?,
    })
}
