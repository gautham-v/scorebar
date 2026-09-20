//! The bridge between the views' [`SnapshotProvider`] and everything that
//! fetches, caches or persists.
//!
//! A refresh is eight-odd blocking http calls against Sleeper's cdn, and the
//! popover renders on the main thread, so every fetch here has the shape
//! claudebar's store provider uses: hand the work to
//! `cx.background_executor()`, and when it lands hop back to the main thread
//! and run the change hook, which re-renders the popover and re-draws the menu
//! bar item. Nothing in this file blocks the ui.
//!
//! Four rules the ui depends on, and they are the reason this file is not just
//! a call to [`scorebar_core::snapshot`]:
//!
//! - **The first paint is the cached week.** The snapshot is read from
//!   `~/Library/Caches/scorebar` in [`StoreProvider::new`], before anything is
//!   fetched, so the menu bar has a score in it the instant the app launches
//!   and the popover is never empty.
//! - **A failed fetch never blanks the ui.** The last good snapshot stays
//!   where it is and the failure is recorded beside it; the popover dims the
//!   numbers and says when they are from. That is why [`SnapshotProvider`]
//!   asks for the snapshot and the error separately.
//! - **Sixty seconds is a floor, not a preference.** Sleeper serves the live
//!   endpoints with `s-maxage=60`, so a poll faster than that re-reads
//!   Cloudflare's copy of the same numbers. The settings' interval is clamped
//!   up to [`MIN_FETCH_GAP`] here whatever the file says.
//! - **The player dictionary is fetched once a day.** It is 14 MB and it is
//!   the only way to turn `"4984"` into a name; see [`crate::cache`].
//!
//! # Why the fetch loop is written out rather than delegated
//!
//! [`scorebar_core::snapshot`] does exactly this walk and returns a
//! [`Snapshot`] — but a [`LeagueCard`](scorebar_core::LeagueCard) has no
//! lineup in it, and the matchups the lineup is built from are dropped on the
//! way out. The detail window needs them. Fetching them a second time would
//! mean another `matchups` call per league per minute for payloads that were
//! already in hand, so the walk lives here and calls core's own
//! [`league_card`] for the arithmetic. Core still owns every decision — what
//! "projected" means, who has not played, what a week of zeros is; what is
//! here is the loop and the bookkeeping.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::AsyncApp;
use scorebar_core::{is_yet_to_play, league_card, LeagueCard, Snapshot};
use sleeper::{
    pair_for_roster, LeagueId, Matchup, PlayerId, PlayerIndex, Position, Sleeper, UserId,
};

use crate::cache;
use crate::settings::{MenuBarTitle, Settings};
use crate::status_item::MenuBarState;
use crate::ui::provider::SnapshotProvider;
use crate::ui::window::{LeagueDetail, PlayerLine, SlotRow};

/// The least time between two fetches, and the floor the settings' interval is
/// clamped up to.
///
/// Sleeper's live endpoints are served with `s-maxage=60`. Asking again sooner
/// spends a request to be handed the same numbers, so every popover open does
/// not get its own round trip — [`StoreProvider::refresh_if_stale`] is what
/// opens call, and it reuses anything younger than this.
pub const MIN_FETCH_GAP: Duration = Duration::from_secs(60);

/// The entries in `roster_positions` that are not a starting slot. Sleeper
/// lists the starting lineup first and then pads with these, so dropping them
/// leaves the slots in the order the lineup is submitted in.
const NON_STARTING_SLOTS: [&str; 3] = ["BN", "IR", "TAXI"];

/// Called on the main thread after any background task finishes.
pub type OnChange = Rc<dyn Fn(&mut gpui::App)>;

/// Everything a fetch writes and the views read. Behind a mutex because the
/// background executor's threads are the writers and the main thread is the
/// only reader.
#[derive(Default)]
struct Week {
    /// The last week that landed. Kept across a failed fetch, so a blip of
    /// network trouble shows stale scores plus a muted line rather than an
    /// empty popover.
    snapshot: Option<Snapshot>,
    /// The starting lineups, by league. Filled by the same fetch that fills
    /// `snapshot`, and read only when the detail window is open.
    lineups: HashMap<LeagueId, Vec<SlotRow>>,
    /// What the last fetch failed with, already phrased for the popover.
    error: Option<String>,
}

/// A [`SnapshotProvider`] over the real Sleeper api and the cache directory.
pub struct StoreProvider {
    week: Arc<Mutex<Week>>,
    cx: AsyncApp,
    on_change: RefCell<Option<OnChange>>,
    /// A fetch is already out; a second popover open should not start another.
    fetching: Arc<Mutex<bool>>,
    /// When the last fetch was started, so opens closer together than
    /// [`MIN_FETCH_GAP`] reuse what is already on screen.
    last_fetch: Arc<Mutex<Option<Instant>>>,
    /// The player dictionary, once something has needed it. Held in memory as
    /// well as on disk so a refresh with the detail window open does not read
    /// a few hundred kilobytes of json off the disk every minute.
    players: Arc<Mutex<Option<PlayerIndex>>>,
    /// The user's choices, read once at construction and rewritten whenever
    /// the Settings section changes one.
    settings: RefCell<Settings>,
    /// A complaint about the config file — malformed on load, or unwritable on
    /// save. Shown as the popover's notice line, but never in place of a fetch
    /// error, which is the more urgent of the two.
    settings_note: RefCell<Option<String>>,
}

impl StoreProvider {
    /// Build the provider. Nothing is fetched until [`Self::refresh`] or
    /// [`Self::refresh_if_stale`] runs, so this never blocks and never fails.
    ///
    /// The two files it does read are small and it reads them here, on the
    /// main thread, because everything downstream — the first menu bar title
    /// included — needs them before the first fetch lands.
    pub fn new(cx: &AsyncApp) -> Self {
        let (settings, note) = Settings::load();
        let week = Week {
            snapshot: cache::load_snapshot(),
            ..Week::default()
        };
        Self {
            week: Arc::new(Mutex::new(week)),
            cx: cx.clone(),
            on_change: RefCell::new(None),
            fetching: Arc::new(Mutex::new(false)),
            last_fetch: Arc::new(Mutex::new(None)),
            players: Arc::new(Mutex::new(None)),
            settings: RefCell::new(settings),
            settings_note: RefCell::new(note),
        }
    }

    /// How long `main.rs` should wait before the next poll: whatever the
    /// settings ask for, never faster than [`MIN_FETCH_GAP`].
    ///
    /// Read once per tick rather than once at startup, so a change made in the
    /// Settings section takes effect on the next interval instead of on the
    /// next launch.
    pub fn refresh_interval(&self) -> Duration {
        self.settings.borrow().refresh_interval().max(MIN_FETCH_GAP)
    }

    /// Install the main-thread hook run after every background task.
    pub fn set_on_change(&self, hook: OnChange) {
        *self.on_change.borrow_mut() = Some(hook);
    }

    /// Fetch now, whatever the last one was. This is the Refresh row and the
    /// `r` key: the user asked, so the cdn floor is not a reason to say no.
    pub fn refresh_now(&self) {
        self.fetch(true);
    }

    /// Fetch unless one went out within [`MIN_FETCH_GAP`]. This is what a
    /// popover open and the poll timer call — neither is a reason to spend a
    /// request on numbers the cdn has not refreshed yet.
    pub fn refresh_if_stale(&self) {
        self.fetch(false);
    }

    /// What the menu bar item should draw.
    pub fn menu_bar_state(&self) -> MenuBarState {
        menu_bar_state_for(
            self.week.lock().unwrap().snapshot.as_ref(),
            &self.settings.borrow(),
        )
    }

    /// Every league, with whatever lineup has been fetched for it — what the
    /// detail window is opened on and what a refresh pushes back into it.
    ///
    /// A league whose lineup has not landed yet gets an empty one rather than
    /// being left out: the header is the part worth seeing first, and the rows
    /// appear underneath it when the fetch returns.
    pub fn details(&self) -> Vec<LeagueDetail> {
        let week = self.week.lock().unwrap();
        let Some(snapshot) = &week.snapshot else {
            return Vec::new();
        };
        snapshot
            .leagues
            .iter()
            .map(|card| LeagueDetail {
                card: card.clone(),
                slots: week
                    .lineups
                    .get(&card.league_id)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect()
    }

    /// Run the change hook on the main thread, but never inside the caller's
    /// own update.
    ///
    /// [`Self::set_settings`] is called from a popover row's click handler,
    /// which already holds both the `App` borrow and the popover entity's
    /// lease; reaching for either from there panics. Handing the hook to the
    /// foreground executor lands it on the next turn of the main loop, when
    /// the borrow and the lease are free again — and a fetch, which finishes
    /// on a background thread, is happy either way.
    fn notify_change(&self) {
        let Some(hook) = self.on_change.borrow().clone() else {
            return;
        };
        self.cx
            .spawn(async move |cx: &mut AsyncApp| {
                let _ = cx.update(|cx| hook(cx));
            })
            .detach();
    }

    /// Run the whole refresh off the main thread, then fire the change hook.
    ///
    /// `force` skips the [`MIN_FETCH_GAP`] check but not the in-flight check:
    /// two overlapping refreshes would race to write the same snapshot, and
    /// the second one has nothing new to say anyway.
    fn fetch(&self, force: bool) {
        // With no username there is nothing to ask for. The popover already
        // says so in its own words, so this is not an error — it is the state
        // the app ships in.
        let Some(username) = self.settings.borrow().username().map(str::to_owned) else {
            return;
        };

        // The guard is dropped before the work is spawned, so a refresh that
        // arrives while one is out is dropped rather than queued: the next
        // tick or popover open picks the new numbers up anyway.
        {
            let mut fetching = self.fetching.lock().unwrap();
            if *fetching {
                return;
            }
            let mut last = self.last_fetch.lock().unwrap();
            if !force && last.is_some_and(|at| at.elapsed() < MIN_FETCH_GAP) {
                return;
            }
            *last = Some(Instant::now());
            *fetching = true;
        }

        let week = self.week.clone();
        let fetching = self.fetching.clone();
        let players = self.players.clone();
        let hook = self.on_change.borrow().clone();
        self.cx
            .spawn(async move |cx: &mut AsyncApp| {
                cx.background_executor()
                    .spawn(async move {
                        run_fetch(&username, &week, &players);
                        *fetching.lock().unwrap() = false;
                    })
                    .await;
                if let Some(hook) = hook {
                    let _ = cx.update(|cx| hook(cx));
                }
            })
            .detach();
    }

    /// Throw away everything fetched under the previous username.
    ///
    /// Scores, lineups and the cached snapshot all belong to one account; a
    /// popover that kept showing the old leagues while the new ones loaded
    /// would be showing somebody else's week. The player dictionary is not
    /// account-scoped and survives.
    fn invalidate(&self) {
        {
            let mut week = self.week.lock().unwrap();
            week.snapshot = None;
            week.lineups.clear();
            week.error = None;
        }
        *self.last_fetch.lock().unwrap() = None;
        if let Some(path) = cache::snapshot_path() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl SnapshotProvider for StoreProvider {
    fn snapshot(&self) -> Option<Snapshot> {
        self.week.lock().unwrap().snapshot.clone()
    }

    fn is_fetching(&self) -> bool {
        *self.fetching.lock().unwrap()
    }

    /// The fetch's complaint, or — when the fetch has nothing to say — the
    /// config file's. A broken `config.toml` is worth one line, but an error
    /// that is keeping the scores off the screen is the more urgent of the
    /// two and owns that line while it lasts.
    fn error(&self) -> Option<String> {
        self.week
            .lock()
            .unwrap()
            .error
            .clone()
            .or_else(|| self.settings_note.borrow().clone())
    }

    fn refresh(&self) {
        self.refresh_now();
    }

    fn settings(&self) -> Settings {
        self.settings.borrow().clone()
    }

    /// Take the new settings, write them out, and run the change hook so the
    /// menu bar item is re-titled and the popover re-rendered.
    ///
    /// The settings are applied whether or not the write succeeds — refusing a
    /// click because a directory is read-only would be the wrong trade — and a
    /// failed write becomes the notice line. A successful one clears whatever
    /// the notice was saying about the file, including a load-time complaint
    /// this write has just fixed.
    ///
    /// A changed username is the one setting that is not only cosmetic: the
    /// week on screen belongs to the old account, so it is dropped and a fetch
    /// starts immediately rather than waiting for the next tick.
    fn set_settings(&self, settings: Settings) {
        let renamed = self.settings.borrow().username() != settings.username();
        *self.settings.borrow_mut() = settings;
        *self.settings_note.borrow_mut() = self.settings.borrow().save().err();
        if renamed {
            self.invalidate();
            self.fetch(true);
        }
        self.notify_change();
    }
}

// ── The fetch ────────────────────────────────────────────────────────────────

/// One whole refresh, on a background thread: the week, the lineups, the
/// cache write.
///
/// Everything here is best effort in the sense that nothing it fails at is
/// allowed to clear what is already on screen. A failed week leaves the old
/// one up with a sentence beside it; a failed player dictionary leaves the
/// lineups as ids rather than losing the scores.
fn run_fetch(username: &str, week: &Arc<Mutex<Week>>, players: &Arc<Mutex<Option<PlayerIndex>>>) {
    let api = match Sleeper::new() {
        Ok(api) => api,
        Err(error) => {
            week.lock().unwrap().error = Some(error.to_string());
            return;
        }
    };

    let fetched = match fetch_week(&api, username) {
        Ok(fetched) => fetched,
        Err(error) => {
            week.lock().unwrap().error = Some(error.to_string());
            return;
        }
    };

    cache::save_snapshot(&fetched.snapshot);

    let index = player_index(&api, players, &fetched.rostered);
    let lineups = fetched
        .lineups
        .iter()
        .map(|raw| (raw.league_id.clone(), slot_rows(raw, index.as_ref())))
        .collect();

    let mut week = week.lock().unwrap();
    week.snapshot = Some(fetched.snapshot);
    week.lineups = lineups;
    week.error = None;
}

/// A week plus everything the lineups are built from.
struct Fetched {
    snapshot: Snapshot,
    lineups: Vec<RawLineup>,
    /// Every player on either roster in every league — the set the player
    /// dictionary is trimmed to before it is cached.
    rostered: HashSet<PlayerId>,
}

/// One league's lineups, still as ids. Turned into [`SlotRow`]s once the
/// player dictionary is in hand, which is a separate step because the
/// dictionary is fetched at most once a day and these are fetched every
/// minute.
struct RawLineup {
    league_id: LeagueId,
    /// The league's own `roster_positions`, bench entries and all.
    positions: Vec<String>,
    mine: Vec<(PlayerId, f32)>,
    theirs: Vec<(PlayerId, f32)>,
}

/// Walk Sleeper the way [`scorebar_core::snapshot`] does, keeping the matchups
/// on the way out. See this module's header for why the walk is here.
fn fetch_week(api: &Sleeper, username: &str) -> scorebar_core::Result<Fetched> {
    let state = api.nfl_state()?;
    let season = state.season_year().unwrap_or_default();
    let week = state.week;

    let user = api.user(username).map_err(|error| match error {
        sleeper::Error::NotFound { .. } => scorebar_core::Error::UnknownUsername {
            username: username.to_owned(),
        },
        other => scorebar_core::Error::Sleeper(other),
    })?;

    let leagues = api.leagues(&user.user_id, season)?;
    if leagues.is_empty() {
        return Err(scorebar_core::Error::NoLeagues { season });
    }

    // Undocumented endpoint: no projections is a worse scoreboard, not a
    // failed refresh. Same call and same reasoning as core's.
    let projections = api
        .projections(season, week, &Position::ALL)
        .unwrap_or_default();

    let mut cards = Vec::with_capacity(leagues.len());
    let mut lineups = Vec::with_capacity(leagues.len());
    let mut rostered = HashSet::new();

    for league in &leagues {
        let members = api.league_users(&league.league_id)?;
        let rosters = api.rosters(&league.league_id)?;
        let matchups = api.matchups(&league.league_id, week)?;

        let Some(card) = league_card(
            league,
            week,
            &rosters,
            &members,
            &matchups,
            &user.user_id,
            &projections,
        ) else {
            continue;
        };

        if let Some(raw) = raw_lineup(league, &rosters, &matchups, &user.user_id) {
            for (player, _) in raw.mine.iter().chain(raw.theirs.iter()) {
                rostered.insert(player.clone());
            }
            lineups.push(raw);
        }
        cards.push(card);
    }

    Ok(Fetched {
        snapshot: Snapshot {
            week,
            season,
            fetched_at: now_unix(),
            leagues: cards,
        },
        lineups,
        rostered,
    })
}

/// One league's two lineups, straight off the matchups payload.
///
/// `None` when the user owns no roster in this league or the league scheduled
/// them nothing this week — the same two cases that leave
/// [`LeagueCard::opponent`] empty, and neither has a lineup to draw.
fn raw_lineup(
    league: &sleeper::League,
    rosters: &[sleeper::Roster],
    matchups: &[Matchup],
    user_id: &UserId,
) -> Option<RawLineup> {
    let mine = rosters
        .iter()
        .find(|roster| roster.owner_id.as_ref() == Some(user_id))?;
    let pair = pair_for_roster(matchups, mine.roster_id)?;
    Some(RawLineup {
        league_id: league.league_id.clone(),
        positions: league.roster_positions.clone(),
        mine: pair.home.starter_points(),
        theirs: pair.away.map(Matchup::starter_points).unwrap_or_default(),
    })
}

/// Seconds since the unix epoch, which is how [`Snapshot::fetched_at`] is
/// spelled. A clock set before 1970 reads as the epoch rather than panicking.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

// ── The player dictionary ────────────────────────────────────────────────────

/// The player dictionary, from memory, then the cache, then the network.
///
/// The network copy is 14 MB and Sleeper asks for it at most once a day, so
/// the cached one is used for as long as [`cache::PLAYER_INDEX_TTL`] allows
/// and the fetched one is trimmed to `rostered` before it is written — a few
/// hundred entries out of twelve thousand.
///
/// `None` means the names are not available this time round, which costs the
/// lineup rows their names and nothing else: [`PlayerIndex::short_name`] is
/// not reached, and [`slot_rows`] prints the raw ids instead. One consequence
/// of the trim is worth knowing: a player added to a roster after the index
/// was cached is not in it, and prints as his id until the day rolls over.
fn player_index(
    api: &Sleeper,
    players: &Arc<Mutex<Option<PlayerIndex>>>,
    rostered: &HashSet<PlayerId>,
) -> Option<PlayerIndex> {
    if let Some(index) = players.lock().unwrap().clone() {
        return Some(index);
    }
    if let Some(index) = cache::load_player_index() {
        *players.lock().unwrap() = Some(index.clone());
        return Some(index);
    }
    let mut index = match api.players() {
        Ok(index) => index,
        Err(error) => {
            eprintln!("scorebar: could not fetch the player dictionary: {error}");
            return None;
        }
    };
    index.retain(rostered.iter().cloned());
    cache::save_player_index(&index);
    *players.lock().unwrap() = Some(index.clone());
    Some(index)
}

// ── Turning ids into rows ────────────────────────────────────────────────────

/// One league's [`SlotRow`]s: the starting slots in lineup order, with both
/// managers' players in them.
///
/// The slots come from the league's own `roster_positions` with the bench
/// entries dropped, which is what makes a superflex league say `SUPER_FLEX`
/// rather than this view guessing at `FLEX`. A side with fewer starters than
/// slots — a bye, or a lineup submitted short — leaves that half of the row
/// empty rather than shifting everything up.
fn slot_rows(raw: &RawLineup, index: Option<&PlayerIndex>) -> Vec<SlotRow> {
    raw.positions
        .iter()
        .filter(|position| is_starting_slot(position))
        .enumerate()
        .map(|(slot, position)| SlotRow {
            position: position.clone(),
            mine: player_line(raw.mine.get(slot), index),
            theirs: player_line(raw.theirs.get(slot), index),
        })
        .collect()
}

/// Whether an entry in `roster_positions` is a slot somebody starts in.
fn is_starting_slot(position: &str) -> bool {
    !NON_STARTING_SLOTS.contains(&position)
}

/// One player's row, or `None` for a slot left empty.
///
/// `points` is `None` for a starter who has not scored, which the window draws
/// as an en dash rather than as `0.00`. That is core's
/// [`is_yet_to_play`] rule, approximation and all: a genuine zero reads as a
/// player still to come until the week is marked final. Drawing the dash is
/// the better half of being wrong — `0.00` beside a player who has not kicked
/// off is a claim, and a dash is not.
fn player_line(
    starter: Option<&(PlayerId, f32)>,
    index: Option<&PlayerIndex>,
) -> Option<PlayerLine> {
    let (player, points) = starter?;
    if player.is_empty_slot() {
        return None;
    }
    let entry = index.and_then(|index| index.get(player));
    Some(PlayerLine {
        name: match index {
            Some(index) => index.short_name(player),
            None => player.to_string(),
        },
        team: entry
            .and_then(|entry| entry.team.clone())
            .unwrap_or_default(),
        points: (!is_yet_to_play(player, *points)).then_some(*points),
        // Not the kickoff time or the live stat line the field is for — see
        // `PlayerLine::status` — but the one thing about a player's afternoon
        // that the matchups payload's neighbours do carry.
        status: entry
            .and_then(|entry| entry.injury_status.clone())
            .unwrap_or_default(),
    })
}

// ── The menu bar title ───────────────────────────────────────────────────────

/// What the menu bar item draws for a snapshot and the user's choice of title.
///
/// A free function so the mapping — the only part of this file that is pure —
/// is exercised by the tests below rather than reimplemented by them.
///
/// No snapshot is [`MenuBarState::Idle`]: a faded glyph, which is what the
/// system items do when they have nothing to say. Everything else is
/// [`MenuBarState::Live`], including the glyph-only choice, because the item
/// has something to report and the user has merely asked it not to print the
/// numbers.
pub fn menu_bar_state_for(snapshot: Option<&Snapshot>, settings: &Settings) -> MenuBarState {
    let Some(snapshot) = snapshot else {
        return MenuBarState::Idle;
    };
    if snapshot.leagues.is_empty() {
        return MenuBarState::Idle;
    }
    let label = match settings.menu_bar_title {
        MenuBarTitle::Margin => snapshot.closest_game().map(margin_line),
        MenuBarTitle::ClosestGame => snapshot.closest_game().map(score_line),
        MenuBarTitle::Record => Some(week_record(&snapshot.leagues)),
        MenuBarTitle::GlyphOnly => None,
    };
    MenuBarState::Live { label }
}

/// The closest game as a signed margin: `"+31.02"`, `"−38.88"`, or my score
/// on its own in a week with nobody on the other side.
///
/// A real minus sign rather than a hyphen, to match the en dash the score line
/// uses and because a hyphen in front of a decimal reads as a stray dash at
/// menu bar size.
fn margin_line(card: &LeagueCard) -> String {
    match &card.opponent {
        Some(opponent) => {
            let margin = card.me.score - opponent.score;
            if margin < 0.0 {
                format!("\u{2212}{:.2}", margin.abs())
            } else {
                format!("+{margin:.2}")
            }
        }
        None => card.me.score_text(),
    }
}

/// The closest game as one line: `"118.44 – 79.02"`, or just my score in a
/// week with nobody on the other side.
///
/// An en dash with spaces around it, the way a scoreboard sets a score line —
/// and because a hyphen next to two decimal numbers reads as a minus sign.
fn score_line(card: &LeagueCard) -> String {
    match &card.opponent {
        Some(opponent) => format!("{} – {}", card.me.score_text(), opponent.score_text()),
        None => card.me.score_text(),
    }
}

/// How the week is going across every league: `"2–1"`, or `"2–1–1"` when one
/// of them is level.
///
/// This is *this week's* record, not the season's — the point of the choice is
/// a line that stops moving once the games are over, and "how many of my games
/// am I winning" is the smallest true thing the menu bar can say about a
/// Sunday. A league with nobody on the other side is not counted: there is no
/// game to be winning.
fn week_record(leagues: &[LeagueCard]) -> String {
    let (mut wins, mut losses, mut ties) = (0u32, 0u32, 0u32);
    for card in leagues {
        let Some(opponent) = &card.opponent else {
            continue;
        };
        match card.me.score.total_cmp(&opponent.score) {
            std::cmp::Ordering::Greater => wins += 1,
            std::cmp::Ordering::Less => losses += 1,
            std::cmp::Ordering::Equal => ties += 1,
        }
    }
    if ties > 0 {
        format!("{wins}–{losses}–{ties}")
    } else {
        format!("{wins}–{losses}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scorebar_core::{Side, WeekState};
    use sleeper::RosterId;

    fn side(score: f32) -> Side {
        Side {
            roster_id: RosterId(1),
            team_name: "Team".to_owned(),
            record: "1-0".to_owned(),
            score,
            projected: score + 20.0,
            yet_to_play: 2,
        }
    }

    fn card(name: &str, win_probability: f32, mine: f32, theirs: Option<f32>) -> LeagueCard {
        LeagueCard {
            league_id: LeagueId::from(name),
            name: name.to_owned(),
            me: side(mine),
            opponent: theirs.map(side),
            win_probability,
            state: WeekState::InProgress,
        }
    }

    fn snapshot(leagues: Vec<LeagueCard>) -> Snapshot {
        Snapshot {
            week: 2,
            season: 2025,
            fetched_at: 1_757_530_800,
            leagues,
        }
    }

    fn with(title: MenuBarTitle) -> Settings {
        Settings {
            menu_bar_title: title,
            ..Settings::default()
        }
    }

    fn label(state: &MenuBarState) -> Option<String> {
        match state {
            MenuBarState::Idle => None,
            MenuBarState::Live { label } => label.clone(),
        }
    }

    #[test]
    fn no_snapshot_leaves_the_menu_bar_idle() {
        assert_eq!(
            menu_bar_state_for(None, &Settings::default()),
            MenuBarState::Idle
        );
    }

    /// An account whose leagues all dropped out of the snapshot has nothing to
    /// report, and must not draw an empty score line.
    #[test]
    fn a_snapshot_with_no_leagues_is_idle() {
        assert_eq!(
            menu_bar_state_for(Some(&snapshot(vec![])), &Settings::default()),
            MenuBarState::Idle
        );
    }

    /// The default: the game closest to even, which is the whole reason to put
    /// a fantasy score in the menu bar.
    #[test]
    fn the_default_title_is_the_closest_games_score_line() {
        let week = snapshot(vec![
            card("runaway", 0.93, 118.44, Some(79.02)),
            card("coin toss", 0.54, 82.10, Some(79.66)),
        ]);
        let state = menu_bar_state_for(Some(&week), &Settings::default());
        assert_eq!(label(&state).as_deref(), Some("82.10 – 79.66"));
    }

    /// A week with nobody on the other side is one number, not a score line
    /// with a blank half.
    #[test]
    fn a_league_with_no_opponent_shows_one_score() {
        let week = snapshot(vec![card("bye", 1.0, 104.30, None)]);
        let state = menu_bar_state_for(Some(&week), &Settings::default());
        assert_eq!(label(&state).as_deref(), Some("104.30"));
    }

    #[test]
    fn the_record_counts_the_leagues_being_won() {
        let week = snapshot(vec![
            card("won", 0.9, 118.44, Some(79.02)),
            card("won too", 0.8, 90.00, Some(88.00)),
            card("lost", 0.1, 56.92, Some(104.30)),
        ]);
        let state = menu_bar_state_for(Some(&week), &with(MenuBarTitle::Record));
        assert_eq!(label(&state).as_deref(), Some("2–1"));
    }

    /// A level game is its own column rather than being rounded into a win or
    /// a loss.
    #[test]
    fn a_level_game_is_counted_as_a_tie() {
        let week = snapshot(vec![
            card("won", 0.9, 118.44, Some(79.02)),
            card("level", 0.5, 90.00, Some(90.00)),
        ]);
        let state = menu_bar_state_for(Some(&week), &with(MenuBarTitle::Record));
        assert_eq!(label(&state).as_deref(), Some("1–0–1"));
    }

    /// A bye is not a game, so it changes no column.
    #[test]
    fn a_bye_is_left_out_of_the_record() {
        let week = snapshot(vec![
            card("won", 0.9, 118.44, Some(79.02)),
            card("bye", 1.0, 104.30, None),
        ]);
        let state = menu_bar_state_for(Some(&week), &with(MenuBarTitle::Record));
        assert_eq!(label(&state).as_deref(), Some("1–0"));
    }

    /// Glyph only still has data behind it, so the item is live and drawn at
    /// full strength — it just prints nothing.
    #[test]
    fn glyph_only_is_live_with_no_label() {
        let week = snapshot(vec![card("any", 0.5, 90.0, Some(90.0))]);
        let state = menu_bar_state_for(Some(&week), &with(MenuBarTitle::GlyphOnly));
        assert_eq!(state, MenuBarState::Live { label: None });
    }

    // ── The lineup ──────────────────────────────────────────────────────────

    fn starters(points: &[(&str, f32)]) -> Vec<(PlayerId, f32)> {
        points
            .iter()
            .map(|(id, points)| (PlayerId::from(*id), *points))
            .collect()
    }

    fn raw(
        positions: &[&str],
        mine: Vec<(PlayerId, f32)>,
        theirs: Vec<(PlayerId, f32)>,
    ) -> RawLineup {
        RawLineup {
            league_id: LeagueId::from("league"),
            positions: positions.iter().map(|slot| (*slot).to_owned()).collect(),
            mine,
            theirs,
        }
    }

    /// The bench is not a lineup. Everything after the starting slots has to
    /// be dropped, or the window draws a dozen empty rows under the real ones.
    #[test]
    fn only_the_starting_slots_become_rows() {
        let rows = slot_rows(
            &raw(
                &["QB", "RB", "SUPER_FLEX", "BN", "BN", "IR", "TAXI"],
                starters(&[("1", 20.0), ("2", 14.5), ("3", 9.0)]),
                starters(&[("4", 18.0), ("5", 11.0), ("6", 7.5)]),
            ),
            None,
        );
        assert_eq!(rows.len(), 3);
        let slots: Vec<&str> = rows.iter().map(|row| row.position.as_str()).collect();
        assert_eq!(slots, vec!["QB", "RB", "SUPER_FLEX"]);
    }

    /// The league's own word for the slot survives: a superflex league says
    /// `SUPER_FLEX`, not this view's guess at it.
    #[test]
    fn the_slot_keeps_the_leagues_own_spelling() {
        let rows = slot_rows(
            &raw(&["SUPER_FLEX"], starters(&[("1", 20.0)]), Vec::new()),
            None,
        );
        assert_eq!(rows[0].position, "SUPER_FLEX");
    }

    /// A bye leaves the other half of every row empty rather than dropping the
    /// rows: the lineup is still worth reading with nobody to compare it to.
    #[test]
    fn a_side_with_no_starters_leaves_half_the_row_empty() {
        let rows = slot_rows(
            &raw(
                &["QB", "RB"],
                starters(&[("1", 20.0), ("2", 14.5)]),
                Vec::new(),
            ),
            None,
        );
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.mine.is_some()));
        assert!(rows.iter().all(|row| row.theirs.is_none()));
    }

    /// The empty-slot marker is not a player. A manager who left a flex open
    /// gets a blank half, not a row named `"0"`.
    #[test]
    fn the_empty_starter_marker_is_not_a_player() {
        let rows = slot_rows(
            &raw(
                &["QB", "FLEX"],
                starters(&[("1", 20.0), (sleeper::EMPTY_STARTER, 0.0)]),
                Vec::new(),
            ),
            None,
        );
        assert!(rows[0].mine.is_some());
        assert!(rows[1].mine.is_none());
    }

    /// A starter on zero has not been shown to have scored zero, so the row
    /// draws a dash. One who has scored carries his number.
    #[test]
    fn a_scoreless_starter_has_no_points_yet() {
        let rows = slot_rows(
            &raw(
                &["QB", "RB"],
                starters(&[("1", 0.0), ("2", 14.5)]),
                Vec::new(),
            ),
            None,
        );
        assert_eq!(rows[0].mine.as_ref().unwrap().points, None);
        assert_eq!(rows[1].mine.as_ref().unwrap().points, Some(14.5));
    }

    /// With no player dictionary the rows still draw — as ids. Losing the
    /// names must not cost the scores.
    #[test]
    fn a_missing_dictionary_falls_back_to_the_id() {
        let rows = slot_rows(&raw(&["QB"], starters(&[("4984", 20.0)]), Vec::new()), None);
        let line = rows[0].mine.as_ref().unwrap();
        assert_eq!(line.name, "4984");
        assert_eq!(line.team, "");
        assert_eq!(line.status, "");
    }

    /// And with one, the row carries the short name, the team and whatever
    /// injury designation the player is under.
    #[test]
    fn a_dictionary_fills_in_the_name_the_team_and_the_designation() {
        let mut players = HashMap::new();
        players.insert(
            PlayerId::from("4984"),
            sleeper::Player {
                player_id: PlayerId::from("4984"),
                first_name: Some("Example".to_owned()),
                last_name: Some("Player".to_owned()),
                position: Some("QB".to_owned()),
                team: Some("SF".to_owned()),
                fantasy_positions: vec!["QB".to_owned()],
                injury_status: Some("Questionable".to_owned()),
                injury_body_part: None,
                status: Some("Active".to_owned()),
                full_name: None,
            },
        );
        let index = PlayerIndex::new(players);
        let rows = slot_rows(
            &raw(&["QB"], starters(&[("4984", 20.0)]), Vec::new()),
            Some(&index),
        );
        let line = rows[0].mine.as_ref().unwrap();
        assert_eq!(line.name, "E. Player");
        assert_eq!(line.team, "SF");
        assert_eq!(line.status, "Questionable");
    }

    /// The floor is the cdn's cache window, whatever the file asks for.
    #[test]
    fn the_poll_interval_never_goes_below_the_cdn_floor() {
        assert_eq!(MIN_FETCH_GAP, Duration::from_secs(60));
        assert_eq!(
            Settings {
                refresh_seconds: 15,
                ..Settings::default()
            }
            .refresh_interval()
            .max(MIN_FETCH_GAP),
            MIN_FETCH_GAP
        );
        assert_eq!(
            Settings {
                refresh_seconds: 900,
                ..Settings::default()
            }
            .refresh_interval()
            .max(MIN_FETCH_GAP),
            Duration::from_secs(900)
        );
    }
}
