//! The seam between the views and everything that fetches, caches or persists.
//!
//! The popover never talks to `scorebar_core::snapshot` or to the cache
//! directly: it holds a [`SnapshotProvider`] and asks it for the week. That is
//! what keeps the whole view tree renderable — and testable — with no
//! username, no network and no menu bar, through [`StubProvider`]. The real
//! implementation over the fetch loop and `crate::cache` satisfies the same
//! trait.
//!
//! The trait is deliberately three questions about the data and two about the
//! user's choices, because that is the whole of what the popover draws:
//!
//! - [`SnapshotProvider::snapshot`] — the last good week, which survives a
//!   failed fetch. There is always something to draw once one has landed.
//! - [`SnapshotProvider::is_fetching`] — whether a fetch is out right now,
//!   which is how the first launch says "checking" rather than "no leagues".
//! - [`SnapshotProvider::error`] — what the last fetch failed with, already
//!   phrased for display. Present *alongside* a snapshot rather than instead
//!   of one: the popover dims the old numbers and says when they are from.
//!
//! Every method takes `&self`. The popover holds the provider behind an `Rc`
//! and implementations use interior mutability, so a fetch landing on the
//! background thread does not need a handle back into the view.

use std::cell::RefCell;

use scorebar_core::{LeagueCard, Side, Snapshot, WeekState};
use sleeper::{LeagueId, RosterId};

use crate::settings::Settings;

/// Everything the popover needs from the layers under it.
pub trait SnapshotProvider {
    /// The last week that landed, if one ever has. Kept through a failed
    /// fetch — the popover would rather show stale numbers, dimmed and dated,
    /// than go blank.
    fn snapshot(&self) -> Option<Snapshot>;

    /// Whether a fetch is in flight. Only interesting while there is nothing
    /// to draw: once a snapshot is up, a refresh in the background is not
    /// worth a line of chrome.
    fn is_fetching(&self) -> bool;

    /// What the last fetch failed with, as a sentence the popover can print.
    /// `None` once a fetch succeeds again.
    fn error(&self) -> Option<String>;

    /// Refetch now — the Refresh row, and the `r` key.
    fn refresh(&self);

    /// The user's choices, as the Settings section draws them.
    fn settings(&self) -> Settings;

    /// Replace them. The implementation persists them and re-renders both the
    /// popover and the menu bar item; a failed write becomes the popover's
    /// notice line rather than a panic.
    fn set_settings(&self, settings: Settings);
}

/// Fixture-backed provider: a fixed snapshot in memory, no network and no
/// files. The preview example and the tests run on this.
pub struct StubProvider {
    inner: RefCell<Inner>,
}

struct Inner {
    snapshot: Option<Snapshot>,
    fetching: bool,
    error: Option<String>,
    /// Kept in memory only: the preview and the tests must never touch the
    /// real `~/.config/scorebar/config.toml`.
    settings: Settings,
}

impl Default for StubProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl StubProvider {
    /// A username set and a live Sunday: the state the design was drawn for.
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(Inner {
                snapshot: Some(fixture_snapshot()),
                fetching: false,
                error: None,
                settings: fixture_settings(),
            }),
        }
    }

    /// First launch with nothing configured: no username, no snapshot, and
    /// the popover opens its Settings section by itself.
    pub fn no_username() -> Self {
        let this = Self::new();
        {
            let mut inner = this.inner.borrow_mut();
            inner.snapshot = None;
            inner.settings.sleeper_username = None;
        }
        this
    }

    /// A username set and the first fetch still out.
    pub fn loading() -> Self {
        let this = Self::new();
        {
            let mut inner = this.inner.borrow_mut();
            inner.snapshot = None;
            inner.fetching = true;
        }
        this
    }

    /// A fetch that failed after a good one: the numbers stay up, dimmed,
    /// under a line saying when they are from.
    pub fn with_error(message: &str) -> Self {
        let this = Self::new();
        this.inner.borrow_mut().error = Some(message.to_owned());
        this
    }

    /// Between seasons: the leagues are there, none of them is playing.
    pub fn between_seasons() -> Self {
        let this = Self::new();
        {
            let mut inner = this.inner.borrow_mut();
            if let Some(snapshot) = &mut inner.snapshot {
                for card in &mut snapshot.leagues {
                    card.opponent = None;
                    card.state = WeekState::NoMatchup;
                    card.win_probability = 1.0;
                }
            }
        }
        this
    }
}

impl SnapshotProvider for StubProvider {
    fn snapshot(&self) -> Option<Snapshot> {
        self.inner.borrow().snapshot.clone()
    }
    fn is_fetching(&self) -> bool {
        self.inner.borrow().fetching
    }
    fn error(&self) -> Option<String> {
        self.inner.borrow().error.clone()
    }
    fn refresh(&self) {}
    fn settings(&self) -> Settings {
        self.inner.borrow().settings.clone()
    }
    fn set_settings(&self, settings: Settings) {
        self.inner.borrow_mut().settings = settings;
    }
}

/// The settings the fixture runs on: a username, so the preview shows the
/// scoreboard rather than the invitation, and defaults for the rest.
pub fn fixture_settings() -> Settings {
    Settings {
        sleeper_username: Some(FIXTURE_USERNAME.to_owned()),
        ..Settings::default()
    }
}

/// The invented manager the fixture belongs to. Nothing here is a real
/// account: the repository is public and no Sleeper username, league or team
/// from a live account appears in it.
pub const FIXTURE_USERNAME: &str = "example_manager";

/// The fixture week: three invented leagues, one comfortably ahead, one that
/// could go either way, one being beaten, and a league name long enough to
/// need its ellipsis.
pub fn fixture_snapshot() -> Snapshot {
    Snapshot {
        week: 2,
        season: 2025,
        // A fixed instant rather than "now": the preview's error mode prints
        // this, and a screenshot of it should say the same thing twice.
        fetched_at: FIXTURE_FETCHED_AT,
        leagues: vec![
            LeagueCard {
                league_id: LeagueId::from("fixture-ahead"),
                name: "Sunday Gravy".to_owned(),
                me: side(1, "Cold Brew Comeback", "2-0", 118.44, 131.6, 3),
                opponent: Some(side(6, "Fourth Down Dogs", "1-1", 79.02, 92.0, 4)),
                win_probability: 0.93,
                state: WeekState::InProgress,
            },
            LeagueCard {
                league_id: LeagueId::from("fixture-close"),
                // Long on purpose: the block's first line has to truncate
                // this and still print the scores in full.
                name: "The Thursday Night Regrets Dynasty League".to_owned(),
                me: side(4, "Play Action Heroes", "1-1", 82.10, 104.9, 5),
                opponent: Some(side(9, "Screen Pass Society", "1-1", 79.66, 101.2, 5)),
                win_probability: 0.54,
                state: WeekState::InProgress,
            },
            LeagueCard {
                league_id: LeagueId::from("fixture-behind"),
                name: "Waiver Wire Wednesday".to_owned(),
                me: side(2, "Hurry Up Offense", "0-2", 56.92, 98.3, 6),
                opponent: Some(side(11, "Garbage Time Gospel", "2-0", 104.30, 137.5, 2)),
                win_probability: 0.11,
                state: WeekState::InProgress,
            },
        ],
    }
}

/// The fixture's "fetched at": a Sunday afternoon, as plain unix seconds.
pub const FIXTURE_FETCHED_AT: i64 = 1_757_530_800;

/// One invented team.
fn side(
    roster_id: u32,
    team_name: &str,
    record: &str,
    score: f32,
    projected: f32,
    yet_to_play: u8,
) -> Side {
    Side {
        roster_id: RosterId(roster_id),
        team_name: team_name.to_owned(),
        record: record.to_owned(),
        score,
        projected,
        yet_to_play,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture has to exercise the three cases the design distinguishes,
    /// or the preview is not showing what it claims to.
    #[test]
    fn the_fixture_is_one_ahead_one_close_and_one_behind() {
        let snapshot = fixture_snapshot();
        assert_eq!(snapshot.week, 2);
        assert_eq!(snapshot.leagues.len(), 3);
        let percents: Vec<u32> = snapshot
            .leagues
            .iter()
            .map(|card| card.win_percent())
            .collect();
        assert_eq!(percents, vec![93, 54, 11]);
        // The closest game is the one the menu bar would pick.
        assert_eq!(
            snapshot.closest_game().map(|card| card.name.as_str()),
            Some("The Thursday Night Regrets Dynasty League")
        );
    }

    /// One name has to be too long for 260px, or the ellipsis is never drawn
    /// in the preview and the truncation is never seen before shipping.
    #[test]
    fn one_league_name_is_long_enough_to_truncate() {
        let snapshot = fixture_snapshot();
        assert!(snapshot.leagues.iter().any(|card| card.name.len() > 30));
    }

    /// Every side carries both scores, so the block draws the same shape in
    /// all three leagues.
    #[test]
    fn every_league_has_an_opponent_and_a_projection() {
        for card in fixture_snapshot().leagues {
            let opponent = card.opponent.expect("fixture leagues are all playing");
            assert!(card.me.projected > card.me.score);
            assert!(opponent.projected > opponent.score);
            assert_eq!(card.state, WeekState::InProgress);
        }
    }

    #[test]
    fn the_stub_states_are_what_they_say() {
        assert!(StubProvider::new().error().is_none());
        assert!(StubProvider::no_username().snapshot().is_none());
        assert!(StubProvider::no_username().settings().username().is_none());
        assert!(StubProvider::loading().is_fetching());
        assert!(StubProvider::loading().snapshot().is_none());

        let failed = StubProvider::with_error("Offline");
        assert_eq!(failed.error().as_deref(), Some("Offline"));
        // The point of the error state: the numbers are still there.
        assert!(failed.snapshot().is_some());

        let idle = StubProvider::between_seasons();
        for card in idle.snapshot().unwrap().leagues {
            assert!(card.opponent.is_none());
            assert_eq!(card.state, WeekState::NoMatchup);
        }
    }

    /// The stub keeps settings in memory, so the preview and the tests can
    /// flip them without writing to the user's config directory.
    #[test]
    fn the_stub_holds_settings_without_a_file() {
        let stub = StubProvider::new();
        assert_eq!(stub.settings().username(), Some(FIXTURE_USERNAME));
        stub.set_settings(Settings {
            refresh_seconds: 300,
            ..fixture_settings()
        });
        assert_eq!(stub.settings().refresh_seconds, 300);
    }
}
