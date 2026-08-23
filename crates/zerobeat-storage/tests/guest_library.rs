use tempfile::tempdir;
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
}
