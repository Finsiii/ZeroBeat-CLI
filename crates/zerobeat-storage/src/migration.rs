use rusqlite::Connection;

use crate::StorageError;

pub(crate) fn migrate(connection: &Connection) -> Result<(), StorageError> {
    connection.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS tracks (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            artist TEXT NOT NULL,
            duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0)
        );

        CREATE TABLE IF NOT EXISTS liked_tracks (
            track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
            liked_at INTEGER NOT NULL DEFAULT (unixepoch())
        );

        CREATE TABLE IF NOT EXISTS play_history (
            track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
            played_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS play_history_recent
            ON play_history(played_at DESC);

        CREATE TABLE IF NOT EXISTS downloads (
            track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
            state TEXT NOT NULL,
            local_path TEXT
        );

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS lyrics (
            track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
            synced INTEGER NOT NULL,
            lines_json TEXT NOT NULL
        );

        ",
    )?;
    let has_thumbnail: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pragma_table_info('tracks') WHERE name = 'thumbnail_url'
        )",
        [],
        |row| row.get(0),
    )?;
    if !has_thumbnail {
        connection.execute("ALTER TABLE tracks ADD COLUMN thumbnail_url TEXT", [])?;
    }
    let has_download_error: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pragma_table_info('downloads') WHERE name = 'error_text'
        )",
        [],
        |row| row.get(0),
    )?;
    if !has_download_error {
        connection.execute("ALTER TABLE downloads ADD COLUMN error_text TEXT", [])?;
    }
    connection.pragma_update(None, "user_version", 5)?;
    Ok(())
}
