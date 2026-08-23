use std::{path::Path, str::FromStr};

use rusqlite::{Connection, OptionalExtension, Row, params};
use zerobeat_core::Track;

use crate::{Download, DownloadState, StorageError, migration};

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        migration::migrate(&connection)?;
        Ok(Self { connection })
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let connection = Connection::open_in_memory()?;
        migration::migrate(&connection)?;
        Ok(Self { connection })
    }

    pub fn set_liked(&self, track: &Track, liked: bool) -> Result<(), StorageError> {
        self.store_track(track)?;
        if liked {
            self.connection.execute(
                "INSERT INTO liked_tracks(track_id) VALUES (?1)
                 ON CONFLICT(track_id) DO NOTHING",
                [&track.id],
            )?;
        } else {
            self.connection
                .execute("DELETE FROM liked_tracks WHERE track_id = ?1", [&track.id])?;
        }
        Ok(())
    }

    pub fn is_liked(&self, track_id: &str) -> Result<bool, StorageError> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM liked_tracks WHERE track_id = ?1)",
            [track_id],
            |row| row.get(0),
        )?)
    }

    pub fn liked_tracks(&self) -> Result<Vec<Track>, StorageError> {
        self.query_tracks(
            "SELECT t.id, t.title, t.artist, t.duration_ms
             FROM liked_tracks l JOIN tracks t ON t.id = l.track_id
             ORDER BY l.liked_at DESC, t.id ASC",
            [],
        )
    }

    pub fn record_play(&self, track: &Track, played_at: i64) -> Result<(), StorageError> {
        self.store_track(track)?;
        self.connection.execute(
            "INSERT INTO play_history(track_id, played_at) VALUES (?1, ?2)
             ON CONFLICT(track_id) DO UPDATE SET played_at = excluded.played_at",
            params![track.id, played_at],
        )?;
        Ok(())
    }

    pub fn recent_tracks(&self, limit: usize) -> Result<Vec<Track>, StorageError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.query_tracks(
            "SELECT t.id, t.title, t.artist, t.duration_ms
             FROM play_history h JOIN tracks t ON t.id = h.track_id
             ORDER BY h.played_at DESC, t.id ASC LIMIT ?1",
            [limit],
        )
    }

    pub fn set_download(
        &self,
        track: &Track,
        state: DownloadState,
        local_path: Option<&str>,
    ) -> Result<(), StorageError> {
        self.store_track(track)?;
        self.connection.execute(
            "INSERT INTO downloads(track_id, state, local_path) VALUES (?1, ?2, ?3)
             ON CONFLICT(track_id) DO UPDATE SET
                state = excluded.state,
                local_path = excluded.local_path",
            params![track.id, state.as_str(), local_path],
        )?;
        Ok(())
    }

    pub fn download(&self, track_id: &str) -> Result<Option<Download>, StorageError> {
        let raw = self
            .connection
            .query_row(
                "SELECT t.id, t.title, t.artist, t.duration_ms, d.state, d.local_path
                 FROM downloads d JOIN tracks t ON t.id = d.track_id
                 WHERE d.track_id = ?1",
                [track_id],
                |row| {
                    Ok((
                        track_from_row(row)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?;

        raw.map(|(track, state, local_path)| {
            Ok(Download {
                track,
                state: DownloadState::from_str(&state)?,
                local_path,
            })
        })
        .transpose()
    }

    fn store_track(&self, track: &Track) -> Result<(), StorageError> {
        let duration_ms =
            i64::try_from(track.duration_ms).map_err(|_| StorageError::DurationOutOfRange)?;
        self.connection.execute(
            "INSERT INTO tracks(id, title, artist, duration_ms) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                artist = excluded.artist,
                duration_ms = excluded.duration_ms",
            params![track.id, track.title, track.artist, duration_ms],
        )?;
        Ok(())
    }

    fn query_tracks<P>(&self, sql: &str, params: P) -> Result<Vec<Track>, StorageError>
    where
        P: rusqlite::Params,
    {
        let mut statement = self.connection.prepare_cached(sql)?;
        let rows = statement.query_map(params, track_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

fn track_from_row(row: &Row<'_>) -> rusqlite::Result<Track> {
    let duration_ms = row.get::<_, i64>(3)?;
    let duration_ms = u64::try_from(duration_ms).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(Track::new(
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        duration_ms,
    ))
}
