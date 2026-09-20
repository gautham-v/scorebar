//! What scorebar keeps on disk between runs, under
//! `~/Library/Caches/scorebar`.
//!
//! Two things live here, and they are cached for opposite reasons.
//!
//! The **snapshot** is small and written on every successful fetch. Without it
//! a fresh launch shows an empty menu bar until the first round trip comes
//! back — four calls per league against a cdn — and a launch with no network
//! shows nothing at all. With it the previous week's numbers are up before the
//! first fetch is even sent, and the fetch merely refreshes them.
//!
//! The **player index** is large and written about once a day. Sleeper's
//! player dictionary is a single 14 MB json object with no per-player endpoint
//! and no way to ask for a subset, and Sleeper's own guidance is to pull it
//! once a day; see [`sleeper::PlayerIndex`]. Trimmed to the players actually
//! on a roster it is a few hundred entries, and a [`load_within`] read is what
//! lets the caller say "give me this if it is younger than a day" without
//! stat-ing the file itself.
//!
//! Neither file holds anything private: public league ids, public scores, and
//! public player names. There is nothing to keep secret because scorebar has
//! no credentials.
//!
//! Errors are logged and otherwise ignored throughout. A cache is a
//! convenience, never the source of truth, and nothing in here should be able
//! to stop the app starting.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use sleeper::PlayerIndex;

use crate::settings::APP_DIR_NAME;

/// How long a cached player index is good for. Sleeper asks for once a day and
/// serves the dictionary with a ten minute cdn cache; a day is the cadence
/// that respects the first and makes the second irrelevant.
pub const PLAYER_INDEX_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// `~/Library/Caches/scorebar`, or `None` on the odd machine with no cache
/// directory.
pub fn dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join(APP_DIR_NAME))
}

/// `~/Library/Caches/scorebar/snapshot.json`: the last good week of scores.
pub fn snapshot_path() -> Option<PathBuf> {
    dir().map(|dir| dir.join("snapshot.json"))
}

/// `~/Library/Caches/scorebar/players.json`: the trimmed player dictionary.
pub fn player_index_path() -> Option<PathBuf> {
    dir().map(|dir| dir.join("players.json"))
}

/// How long ago a cached file was written, or `None` if it is not there or the
/// filesystem will not say.
///
/// A clock that has gone backwards since the write — a laptop that crossed a
/// timezone, or an ntp correction — makes `elapsed` fail rather than return a
/// negative age. That reads as "no age", which makes [`load_within`] treat the
/// file as stale and refetch, which is the safe way to be wrong.
pub fn age(path: &Path) -> Option<Duration> {
    fs::metadata(path).ok()?.modified().ok()?.elapsed().ok()
}

/// Read a cached value. A missing, unreadable or unparseable file is simply
/// "no cache": the fetch that follows is what matters.
///
/// Generic over the payload rather than typed to one struct, because the two
/// things scorebar caches have nothing in common but json — and because the
/// snapshot type lives in `scorebar-core`, which has no business knowing that
/// a cache directory exists.
pub fn load<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// The same, but only if the file was written less than `ttl` ago.
///
/// This is the read for anything with a refresh cadence of its own: the caller
/// says how fresh it needs the value to be, gets `None` when the file is past
/// that, and refetches. A file whose age cannot be read counts as stale.
pub fn load_within<T: DeserializeOwned>(path: &Path, ttl: Duration) -> Option<T> {
    match age(path) {
        Some(age) if age <= ttl => load(path),
        _ => None,
    }
}

/// Write a value, creating the cache directory if it is not there yet. Errors
/// are logged and swallowed.
pub fn save<T: Serialize>(path: &Path, value: &T) {
    if let Err(error) = save_to(path, value) {
        eprintln!("scorebar: could not write {}: {error}", path.display());
    }
}

/// Write a value atomically: to a sibling temporary file, then rename over the
/// target.
///
/// Rename within a directory is atomic, so a reader either sees the whole old
/// file or the whole new one. Plain `write` is not: the player index is
/// megabytes, and a crash or a full disk partway through leaves a truncated
/// file that parses as nothing and has to be refetched — at 14 MB, on the
/// user's next launch.
pub fn save_to<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string(value).map_err(io::Error::other)?;
    let temp = path.with_extension("tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, path)
}

/// The cached snapshot, whatever the caller's snapshot type is.
pub fn load_snapshot<T: DeserializeOwned>() -> Option<T> {
    load(&snapshot_path()?)
}

/// Write the snapshot. Called on every successful fetch.
pub fn save_snapshot<T: Serialize>(snapshot: &T) {
    let Some(path) = snapshot_path() else {
        return;
    };
    save(&path, snapshot);
}

/// The cached player index, if there is one and it is younger than
/// [`PLAYER_INDEX_TTL`]. `None` means "fetch the dictionary again".
pub fn load_player_index() -> Option<PlayerIndex> {
    load_within(&player_index_path()?, PLAYER_INDEX_TTL)
}

/// Write the player index. Trim it with
/// [`PlayerIndex::retain`](sleeper::PlayerIndex::retain) first — the whole
/// dictionary is 14 MB and the rostered slice of it is a few hundred entries.
pub fn save_player_index(index: &PlayerIndex) {
    let Some(path) = player_index_path() else {
        return;
    };
    save(&path, index);
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use sleeper::PlayerId;

    use super::*;

    /// A stand-in for whatever `scorebar-core` calls a snapshot: this module
    /// is generic over the payload, so the tests only need *a* payload.
    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Payload {
        week: u16,
        scores: Vec<f32>,
    }

    fn payload() -> Payload {
        Payload {
            week: 3,
            scores: vec![104.32, 65.44],
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("scorebar-cache-test-{}-{name}", std::process::id()))
            .join("snapshot.json")
    }

    fn clean(path: &Path) {
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_saved_value_reads_back_the_same() {
        let path = temp_path("roundtrip");
        save_to(&path, &payload()).unwrap();
        let back: Payload = load(&path).expect("the cache parses");
        assert_eq!(back, payload());
        clean(&path);
    }

    #[test]
    fn a_missing_or_broken_file_is_no_cache() {
        let path = temp_path("broken");
        assert!(load::<Payload>(&path).is_none());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not json").unwrap();
        assert!(load::<Payload>(&path).is_none());
        clean(&path);
    }

    /// The ttl read is the whole point of the player index cache: a file just
    /// written is inside any sane window, and the same file is outside a
    /// zero-length one.
    #[test]
    fn a_ttl_read_returns_a_fresh_file_and_refuses_a_stale_one() {
        let path = temp_path("ttl");
        save_to(&path, &payload()).unwrap();

        let fresh: Option<Payload> = load_within(&path, Duration::from_secs(60));
        assert_eq!(fresh, Some(payload()));

        // Anything written before now is older than "no time at all".
        let stale: Option<Payload> = load_within(&path, Duration::ZERO);
        assert!(stale.is_none());
        clean(&path);
    }

    /// A file that is not there has no age, and no age is stale rather than
    /// fresh — otherwise a missing cache would read as infinitely young.
    #[test]
    fn a_missing_file_has_no_age_and_never_counts_as_fresh() {
        let path = temp_path("ageless");
        assert!(age(&path).is_none());
        assert!(load_within::<Payload>(&path, Duration::from_secs(86_400)).is_none());
    }

    /// The write creates its directory; nothing above it has to.
    #[test]
    fn writing_creates_the_cache_directory() {
        let path = temp_path("mkdir");
        assert!(!path.parent().unwrap().exists());
        save_to(&path, &payload()).unwrap();
        assert!(path.exists());
        clean(&path);
    }

    /// The temporary file the atomic write goes through must not survive it.
    #[test]
    fn an_atomic_write_leaves_no_temporary_file_behind() {
        let path = temp_path("atomic");
        save_to(&path, &payload()).unwrap();
        assert!(!path.with_extension("tmp").exists());
        clean(&path);
    }

    /// The player index round-trips through the same machinery the snapshot
    /// uses, which is the reason the read and write are generic.
    #[test]
    fn a_player_index_round_trips_through_the_cache() {
        let path = temp_path("players");
        let index = PlayerIndex::new(HashMap::new());
        save_to(&path, &index).unwrap();
        let back: PlayerIndex = load(&path).expect("the index parses");
        assert!(back.is_empty());
        assert!(back.get(&PlayerId::from("4984")).is_none());
        clean(&path);
    }

    /// The two files are siblings under one directory named for the app, so a
    /// user who wants to clear the cache has one thing to delete.
    #[test]
    fn both_caches_sit_under_one_directory_named_for_the_app() {
        let (Some(dir), Some(snapshot), Some(players)) =
            (dir(), snapshot_path(), player_index_path())
        else {
            return;
        };
        assert!(dir.ends_with("scorebar"));
        assert_eq!(snapshot.parent(), Some(dir.as_path()));
        assert_eq!(players.parent(), Some(dir.as_path()));
        assert_ne!(snapshot, players);
    }
}
