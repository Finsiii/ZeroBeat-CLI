use rusqlite::Connection;
use tempfile::tempdir;
use zerobeat_catalog::{Lyrics, LyricsLine};
use zerobeat_core::Track;
use zerobeat_storage::{Database, DownloadState};

fn track(id: &str, title: &str) -> Track {
    Track::new(id, title, "Juicy Luicy", 245_000)
}

#[test]
fn liked_tracks_survive_database_reopen() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("guest.db");
    let song = track("yt:tampar", "Tampar");

    {
        let database = Database::open(&path).unwrap();
        database.set_liked(&song, true).unwrap();
    }

    let database = Database::open(&path).unwrap();
    assert!(database.is_liked(&song.id).unwrap());
    assert_eq!(database.liked_tracks().unwrap(), vec![song]);
}

#[test]
fn stored_tracks_preserve_their_thumbnail() {
    let database = Database::open_in_memory().unwrap();
    let song = track("yt:tampar", "Tampar").with_thumbnail("https://img.example/tampar.jpg");

    database.set_liked(&song, true).unwrap();

    assert_eq!(database.liked_tracks().unwrap(), vec![song]);
}

#[test]
fn version_one_database_is_upgraded_without_losing_tracks() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("guest.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "
            CREATE TABLE tracks (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0)
            );
            CREATE TABLE liked_tracks (
                track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
                liked_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE TABLE play_history (
                track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
                played_at INTEGER NOT NULL
            );
            CREATE TABLE downloads (
                track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
                state TEXT NOT NULL,
                local_path TEXT
            );
            INSERT INTO tracks VALUES ('yt:old', 'Old Song', 'Old Artist', 120000);
            INSERT INTO liked_tracks(track_id) VALUES ('yt:old');
            PRAGMA user_version = 1;
            ",
        )
        .unwrap();
    drop(connection);

    let database = Database::open(&path).unwrap();
    assert_eq!(
        database.liked_tracks().unwrap(),
        vec![Track::new("yt:old", "Old Song", "Old Artist", 120_000)]
    );

    let updated = track("yt:old", "Old Song").with_thumbnail("https://img.example/old.jpg");
    database.set_liked(&updated, true).unwrap();
    assert_eq!(database.liked_tracks().unwrap(), vec![updated]);
}

#[test]
fn play_history_keeps_most_recent_entry_first() {
    let database = Database::open_in_memory().unwrap();
    let first = track("yt:first", "First");
    let second = track("yt:second", "Second");

    database.record_play(&first, 100).unwrap();
    database.record_play(&second, 200).unwrap();
    database.record_play(&first, 300).unwrap();

    assert_eq!(database.recent_tracks(10).unwrap(), vec![first, second]);
}

#[test]
fn download_state_and_path_are_persisted() {
    let database = Database::open_in_memory().unwrap();
    let song = track("yt:tampar", "Tampar");

    database
        .set_download(&song, DownloadState::Downloading, None)
        .unwrap();
    database
        .set_download(&song, DownloadState::Available, Some("/music/tampar.m4a"))
        .unwrap();

    let download = database.download(&song.id).unwrap().unwrap();
    assert_eq!(download.track, song);
    assert_eq!(download.state, DownloadState::Available);
    assert_eq!(download.local_path.as_deref(), Some("/music/tampar.m4a"));
    assert_eq!(download.error, None);
    assert_eq!(database.downloads().unwrap(), vec![download]);
}

#[test]
fn crossfade_preference_survives_database_reopen() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("guest.db");
    let database = Database::open(&path).unwrap();
    assert_eq!(database.crossfade_seconds().unwrap(), 6);
    database.set_crossfade_seconds(9).unwrap();
    drop(database);

    assert_eq!(
        Database::open(path).unwrap().crossfade_seconds().unwrap(),
        9
    );
}

#[test]
fn lyrics_are_cached_for_offline_playback() {
    let database = Database::open_in_memory().unwrap();
    let song = track("yt:tampar", "Tampar");
    let lyrics = Lyrics {
        synced: true,
        lines: vec![LyricsLine {
            start_ms: Some(1_000),
            words: "Entah sudah selasa".into(),
        }],
    };

    database.save_lyrics(&song, &lyrics).unwrap();

    assert_eq!(database.lyrics(&song.id).unwrap(), Some(lyrics));
}
