//! The Sleeper payloads, trimmed to what a scoring ui needs.
//!
//! Three rules shaped everything here.
//!
//! One: nothing uses `deny_unknown_fields`. Sleeper adds keys without warning
//! and a strict struct would turn that into an outage.
//!
//! Two: the parts of a payload that are a user's own league configuration are
//! maps, not structs. `scoring_settings` came back with 42, 44 and 128 keys
//! across three leagues of one account, and `settings` with 50, 51 and 54.
//! Anything that tried to name those fields would be wrong for somebody.
//! [`League::settings`] is the exception and it is deliberate: it names the
//! handful the app actually reads and lets serde drop the rest.
//!
//! Three: ids are newtypes. Sleeper has four id spaces in play at once —
//! `player_id`, `league_id` and `user_id` are quoted digit strings, while
//! `roster_id` is a plain integer — and they are passed to different endpoints
//! in the same function body. [`PlayerId`], [`LeagueId`], [`UserId`] and
//! [`RosterId`] make mixing them a compile error instead of a 404.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

/// The value Sleeper puts in `starters` for a lineup slot nobody filled. It
/// looks like a player id and is not one: it is never in `players` and never a
/// key in `players_points`.
pub const EMPTY_STARTER: &str = "0";

/// Defines a transparent `String` newtype for one of Sleeper's id spaces.
///
/// A macro rather than three hand-written copies, because the only thing that
/// differs between them is the name — and the name is the entire point.
macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// The id as Sleeper spells it, for building a url.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id! {
    /// A player's id in `/v1/players/nfl`. Digits for a person (`"4984"`), a
    /// team abbreviation for a defense (`"BUF"`).
    PlayerId
}

string_id! {
    /// A league's id. Dynasty leagues chain year to year through
    /// `previous_league_id`, so this changes every season.
    LeagueId
}

string_id! {
    /// An account's id. Stable forever, unlike the username, which is why the
    /// app resolves a username once and stores this.
    UserId
}

impl PlayerId {
    /// Whether this is [`EMPTY_STARTER`] rather than a real player. Looking
    /// one of these up in the player dictionary always misses, so the ui has
    /// to check before it renders a row.
    pub fn is_empty_slot(&self) -> bool {
        self.0 == EMPTY_STARTER
    }
}

/// A roster's id **within one league**: 1..=`total_rosters`, and an integer
/// rather than a string. Two different leagues both have a roster 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RosterId(pub u32);

impl fmt::Display for RosterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for RosterId {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

/// A fantasy position, as the projections endpoint spells it in `position[]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Position {
    /// Quarterback.
    QB,
    /// Running back.
    RB,
    /// Wide receiver.
    WR,
    /// Tight end.
    TE,
    /// Kicker.
    K,
    /// Team defense and special teams.
    DEF,
}

impl Position {
    /// The wire spelling, which is also the display spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            Position::QB => "QB",
            Position::RB => "RB",
            Position::WR => "WR",
            Position::TE => "TE",
            Position::K => "K",
            Position::DEF => "DEF",
        }
    }

    /// Every position a starting lineup can hold, which is what a week's
    /// projections call asks for.
    pub const ALL: [Position; 6] = [
        Position::QB,
        Position::RB,
        Position::WR,
        Position::TE,
        Position::K,
        Position::DEF,
    ];
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Deserialize a list that Sleeper sometimes sends as `null` instead of `[]`.
///
/// `reserve`, `taxi` and occasionally `players` come back null on a roster
/// that has none, and every caller wants the same empty vec, so the null dies
/// here rather than in an `Option` every call site has to unwrap.
fn null_as_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

/// Deserialize a map that Sleeper sometimes sends as `null` instead of `{}`.
fn null_as_empty_map<'de, D, K, V>(deserializer: D) -> Result<HashMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: Deserialize<'de> + std::hash::Hash + Eq,
    V: Deserialize<'de>,
{
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

/// `GET /v1/state/nfl` — the clock every other call hangs off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    /// The scoring week. This is the one to pass to
    /// [`crate::Sleeper::matchups`].
    pub week: u8,
    /// The season as a string, because it is a url path segment (`"2026"`).
    pub season: String,
    /// `"pre"`, `"regular"` or `"post"`.
    pub season_type: String,
    /// The week Sleeper's own ui labels as current. It agrees with `week`
    /// mid-season and diverges at the edges, right after a week's games end.
    #[serde(default)]
    pub display_week: Option<u8>,
    /// `YYYY-MM-DD` for the first day of the season.
    #[serde(default)]
    pub season_start_date: Option<String>,
    /// The week within the season type. Equal to `week` during the regular
    /// season; the preseason numbers its own legs.
    #[serde(default)]
    pub leg: Option<u8>,
}

impl State {
    /// The season as a number, for the calls that take one. `None` if Sleeper
    /// ever sends something that is not a year.
    pub fn season_year(&self) -> Option<u16> {
        self.season.parse().ok()
    }
}

/// `GET /v1/user/<username_or_id>` — one account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// The stable id. Everything downstream keys off this, not the username.
    pub user_id: UserId,
    /// The login name. Changeable, and `None` on some legacy accounts.
    #[serde(default)]
    pub username: Option<String>,
    /// The name Sleeper shows. Usually equal to the username.
    pub display_name: String,
    /// A bare 32-character hash, not a url. See [`avatar_url`].
    #[serde(default)]
    pub avatar: Option<String>,
}

impl User {
    /// The full avatar url, or `None` for an account that never set one.
    pub fn avatar_url(&self) -> Option<String> {
        self.avatar.as_deref().map(avatar_url)
    }
}

/// Turn a bare avatar hash into a url.
///
/// Sleeper has two avatar conventions at once: a top-level `avatar` is a bare
/// hash that has to be prefixed, while `metadata.avatar` is already a complete
/// url. Handing the wrong one to an image loader gets a 404, so the
/// conversion lives in exactly one place.
pub fn avatar_url(hash: &str) -> String {
    format!("https://sleepercdn.com/avatars/{hash}")
}

/// A league, from either `GET /v1/league/<id>` or the list at
/// `GET /v1/user/<id>/leagues/nfl/<season>`.
///
/// One struct for both, because the two payloads are identical apart from two
/// fields the list carries and the single-league fetch does not.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct League {
    /// This season's id. A dynasty league gets a new one every year.
    pub league_id: LeagueId,
    /// The league's name, as its members named it.
    pub name: String,
    /// The season as a string, matching [`State::season`].
    pub season: String,
    /// How many teams, which is also the highest [`RosterId`].
    pub total_rosters: u32,
    /// `"pre_draft"`, `"drafting"`, `"in_season"` or `"complete"`.
    pub status: String,
    /// The league's scoring rules: stat key to points per unit. A flat map
    /// because the shape is per-league — 42 to 128 keys in one account's three
    /// leagues. Multiply it against a [`Projection::stats`] to get a
    /// league-accurate projection instead of a generic one.
    #[serde(default, deserialize_with = "null_as_empty_map")]
    pub scoring_settings: HashMap<String, f64>,
    /// The starting lineup, in order, then `"BN"` for each bench slot. This is
    /// the label for each index of [`Matchup::starters`]; the two arrays are
    /// parallel and neither one carries a slot name.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub roster_positions: Vec<String>,
    /// The handful of league settings the app reads. Sleeper sends 50-odd and
    /// the set differs per league, so this names the ones that matter and lets
    /// serde ignore everything else.
    #[serde(default)]
    pub settings: LeagueSettings,
    /// Where the user dragged this league in their own list. Present only in
    /// the leagues list, absent from the single-league fetch.
    #[serde(default)]
    pub display_order: Option<i64>,
    /// The league's most recent transaction, as an unquoted integer. Also
    /// list-only, and null for a league with no transactions yet.
    #[serde(default)]
    pub last_transaction_id: Option<i64>,
}

/// The few `league.settings` keys the app reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LeagueSettings {
    /// The last week Sleeper has finished scoring. The only way to tell a week
    /// that has not kicked off from a week where everyone scored zero: an
    /// unplayed week returns a complete, well-formed payload of zeros.
    pub last_scored_leg: Option<u8>,
    /// The league's current week, which can lag [`State::week`] by a few
    /// minutes at a week boundary.
    pub leg: Option<u8>,
    /// The first week this league scores.
    pub start_week: Option<u8>,
    /// The first week of the playoffs; from here the matchups are bracketed.
    pub playoff_week_start: Option<u8>,
    /// How many teams make the playoffs.
    pub playoff_teams: Option<u8>,
    /// Team count, duplicating [`League::total_rosters`].
    pub num_teams: Option<u8>,
}

/// `GET /v1/league/<id>/users` — one member of a league.
///
/// Distinct from [`User`]: this is an account *as seen inside a league*, which
/// is where the team name lives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeagueUser {
    /// Joins to [`Roster::owner_id`].
    pub user_id: UserId,
    /// The account's global display name, the fallback when there is no team
    /// name.
    pub display_name: String,
    /// A bare avatar hash, as on [`User::avatar`].
    #[serde(default)]
    pub avatar: Option<String>,
    /// Per-league settings. The team name lives in here, not at the top level.
    #[serde(default)]
    pub metadata: LeagueUserMetadata,
}

/// The parts of a [`LeagueUser`]'s metadata worth keeping.
///
/// Sleeper puts notification preferences in the same map, and which keys are
/// present varies by member, so everything is optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LeagueUserMetadata {
    /// What this manager called their team in this league. Absent for anyone
    /// who never set one.
    pub team_name: Option<String>,
    /// A custom avatar, already a **complete url** — unlike
    /// [`LeagueUser::avatar`], which is a bare hash.
    pub avatar: Option<String>,
}

impl LeagueUser {
    /// What to print for this manager: their team name if they set one, their
    /// display name otherwise. The ui should never show a blank team.
    pub fn team_name(&self) -> &str {
        self.metadata
            .team_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(&self.display_name)
    }

    /// The avatar to load: the custom url if there is one, otherwise the
    /// account avatar expanded from its hash.
    pub fn avatar_url(&self) -> Option<String> {
        self.metadata
            .avatar
            .clone()
            .or_else(|| self.avatar.as_deref().map(avatar_url))
    }
}

/// `GET /v1/league/<id>/rosters` — a team's season-long state.
///
/// For the current week's lineup use [`Matchup`] instead: a roster's
/// `starters` and the same week's matchup `starters` can disagree, and the
/// matchup is the live one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Roster {
    /// This roster's number within the league.
    pub roster_id: RosterId,
    /// The manager who owns it. `None` for an orphaned team.
    #[serde(default)]
    pub owner_id: Option<UserId>,
    /// Everyone on the roster, starters and bench together, unordered.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub players: Vec<PlayerId>,
    /// The lineup, positionally parallel to [`League::roster_positions`].
    /// Can contain [`EMPTY_STARTER`] for a slot left open.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub starters: Vec<PlayerId>,
    /// Record and season totals.
    #[serde(default)]
    pub settings: RosterSettings,
}

/// A roster's record and season point totals.
///
/// The points are split across two integers: `fpts: 83` with
/// `fpts_decimal: 10` means 83.10. Sleeper does this to keep the numbers exact
/// over a json round trip, so the two are recombined in
/// [`RosterSettings::points`] rather than anywhere a caller might forget.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RosterSettings {
    /// Wins so far.
    pub wins: u32,
    /// Losses so far.
    pub losses: u32,
    /// Ties so far.
    pub ties: u32,
    /// Whole points scored this season.
    pub fpts: i64,
    /// Hundredths of a point scored this season.
    pub fpts_decimal: i64,
    /// Whole points scored against this roster.
    pub fpts_against: i64,
    /// Hundredths of a point scored against this roster.
    pub fpts_against_decimal: i64,
    /// Whole points the optimal lineup would have scored.
    pub ppts: i64,
    /// Hundredths of a point the optimal lineup would have scored.
    pub ppts_decimal: i64,
}

impl RosterSettings {
    /// Season points scored, with the two halves put back together.
    pub fn points(&self) -> f32 {
        combine_points(self.fpts, self.fpts_decimal)
    }

    /// Season points scored against.
    pub fn points_against(&self) -> f32 {
        combine_points(self.fpts_against, self.fpts_against_decimal)
    }

    /// Season points the best possible lineup would have scored.
    pub fn potential_points(&self) -> f32 {
        combine_points(self.ppts, self.ppts_decimal)
    }

    /// The record as a ui prints it: `"1-0"`, or `"1-0-1"` when there are ties.
    pub fn record(&self) -> String {
        if self.ties == 0 {
            format!("{}-{}", self.wins, self.losses)
        } else {
            format!("{}-{}-{}", self.wins, self.losses, self.ties)
        }
    }
}

/// Recombine Sleeper's split point totals. The decimal half is hundredths.
fn combine_points(whole: i64, decimal: i64) -> f32 {
    whole as f32 + decimal as f32 / 100.0
}

/// `GET /v1/league/<id>/matchups/<week>` — one team's week.
///
/// Both sides of a game appear as separate entries sharing a `matchup_id`;
/// [`pairs`] puts them back together.
///
/// An unplayed week returns a complete payload of zeros rather than an empty
/// array or a 404, so "the games have not started" and "everyone scored zero"
/// are indistinguishable from this alone. [`State::week`] and
/// [`LeagueSettings::last_scored_leg`] are what tell them apart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Matchup {
    /// Whose week this is.
    pub roster_id: RosterId,
    /// The game this roster is in. Both teams share it. `None` on a bye or in
    /// a league that is not scheduling head-to-head that week.
    #[serde(default)]
    pub matchup_id: Option<u32>,
    /// Points so far this week. Equal to the sum of [`Self::starters_points`].
    #[serde(default)]
    pub points: f32,
    /// A commissioner override, which replaces `points` when present.
    #[serde(default)]
    pub custom_points: Option<f32>,
    /// The live lineup, positionally parallel to
    /// [`League::roster_positions`] and to [`Self::starters_points`]. Can
    /// disagree with the same week's [`Roster::starters`]; this one wins.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub starters: Vec<PlayerId>,
    /// Points per starter, **by position, not by player**: index `i` belongs
    /// to `starters[i]`. See [`Self::starter_points`].
    #[serde(default, deserialize_with = "null_as_empty")]
    pub starters_points: Vec<f32>,
    /// Everyone on the roster this week, starters and bench.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub players: Vec<PlayerId>,
    /// Points per player including the bench. Unlike `starters_points` this is
    /// keyed, so it is the safe one to look a single player up in.
    #[serde(default, deserialize_with = "null_as_empty_map")]
    pub players_points: HashMap<PlayerId, f32>,
}

impl Matchup {
    /// The week's score, preferring a commissioner override.
    pub fn score(&self) -> f32 {
        self.custom_points.unwrap_or(self.points)
    }

    /// The lineup as `(player, points)` in slot order, ready to zip against
    /// [`League::roster_positions`] for the slot labels.
    ///
    /// Walks `starters` rather than zipping the two vectors, because they can
    /// be different lengths — an empty slot has shown up as a missing entry in
    /// `starters_points` rather than a zero — and a plain `zip` would silently
    /// truncate the lineup, shifting every slot label after the gap. A starter
    /// with no matching points entry is reported as zero, which is what the ui
    /// would draw anyway. Extra points past the end of `starters` are dropped:
    /// with no player to attach them to there is no row to put them on.
    pub fn starter_points(&self) -> Vec<(PlayerId, f32)> {
        self.starters
            .iter()
            .enumerate()
            .map(|(slot, player)| {
                (
                    player.clone(),
                    self.starters_points.get(slot).copied().unwrap_or(0.0),
                )
            })
            .collect()
    }

    /// Points for one player, bench included. `None` if they were not on this
    /// roster this week.
    pub fn player_points(&self, player: &PlayerId) -> Option<f32> {
        self.players_points.get(player).copied()
    }
}

/// Two rosters playing each other in one week.
///
/// `away` is `None` for a bye, an odd number of teams, or a league that is not
/// pairing teams that week — none of which is an error, so it is an `Option`
/// rather than a reason to drop the row.
#[derive(Debug, Clone, Copy)]
pub struct Pair<'a> {
    /// The id both sides share, or `None` when the entry had no `matchup_id`.
    pub matchup_id: Option<u32>,
    /// One side. For [`pair_for_roster`] this is always the roster asked for.
    pub home: &'a Matchup,
    /// The other side, when there is one.
    pub away: Option<&'a Matchup>,
}

impl<'a> Pair<'a> {
    /// Whether this pairing involves the given roster.
    pub fn contains(&self, roster_id: RosterId) -> bool {
        self.home.roster_id == roster_id
            || self.away.is_some_and(|away| away.roster_id == roster_id)
    }

    /// The other side of the game from the given roster. `None` if that roster
    /// is not in this pairing, or has no opponent.
    pub fn opponent_of(&self, roster_id: RosterId) -> Option<&'a Matchup> {
        if self.home.roster_id == roster_id {
            self.away
        } else if self.away.is_some_and(|away| away.roster_id == roster_id) {
            Some(self.home)
        } else {
            None
        }
    }

    /// How far ahead [`Self::home`] is, negative when it is behind. `None`
    /// without an opponent to compare to.
    pub fn margin(&self) -> Option<f32> {
        self.away.map(|away| self.home.score() - away.score())
    }
}

/// Group a week's matchups into head-to-head pairings.
///
/// Sleeper sends one entry per roster, so a twelve-team league's week is
/// twelve objects that have to be matched on `matchup_id`. Entries keep the
/// order they arrived in, and anything that cannot be paired — a bye, an odd
/// team out, a null `matchup_id` — comes back as a pairing with no `away`
/// rather than being dropped.
pub fn pairs(matchups: &[Matchup]) -> Vec<Pair<'_>> {
    let mut groups: Vec<(Option<u32>, Vec<&Matchup>)> = Vec::new();

    for matchup in matchups {
        // A null matchup_id is not a group: two rosters both missing one are
        // not playing each other, they are each unpaired.
        match matchup.matchup_id {
            None => groups.push((None, vec![matchup])),
            Some(id) => match groups
                .iter_mut()
                .find(|(group_id, _)| *group_id == Some(id))
            {
                Some((_, members)) => members.push(matchup),
                None => groups.push((Some(id), vec![matchup])),
            },
        }
    }

    groups
        .into_iter()
        .flat_map(|(matchup_id, members)| {
            // chunks(2) rather than an exact pair: three rosters sharing an id
            // should not make the third vanish.
            members
                .chunks(2)
                .map(|chunk| Pair {
                    matchup_id,
                    home: chunk[0],
                    away: chunk.get(1).copied(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The pairing containing one roster, oriented so that roster is `home`.
///
/// The orientation is the point: the app always draws the user's team on the
/// left, and [`Pair::margin`] should be positive when they are winning.
pub fn pair_for_roster(matchups: &[Matchup], roster_id: RosterId) -> Option<Pair<'_>> {
    pairs(matchups).into_iter().find_map(|pair| {
        if pair.home.roster_id == roster_id {
            Some(pair)
        } else if pair.away.is_some_and(|away| away.roster_id == roster_id) {
            Some(Pair {
                matchup_id: pair.matchup_id,
                home: pair.away?,
                away: Some(pair.home),
            })
        } else {
            None
        }
    })
}

/// One player's projection for one week, from the undocumented
/// `GET /projections/nfl/<season>/<week>` endpoint.
///
/// Most fields are optional because most entries are not really projections:
/// the endpoint returns every player on file at the position, and the ones
/// nobody projected carry a `stats` map of exactly `{"adp_dd_ppr": 1000.0}`
/// with `team`, `opponent`, `date` and `game_id` all null.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Projection {
    /// Joins to [`Matchup::starters`] and the player dictionary.
    pub player_id: PlayerId,
    /// The week projected.
    pub week: u8,
    /// The season, as a string like everywhere else.
    pub season: String,
    /// Projected stat per key — the same keys as [`League::scoring_settings`],
    /// which is what makes [`Self::points_with`] possible. Includes the
    /// prebaked `pts_std`, `pts_ppr` and `pts_half_ppr` when there is a real
    /// projection.
    #[serde(default, deserialize_with = "null_as_empty_map")]
    pub stats: HashMap<String, f64>,
    /// Who they play, `None` for a free agent or a bye.
    #[serde(default)]
    pub opponent: Option<String>,
    /// Their nfl team, `None` for a free agent.
    #[serde(default)]
    pub team: Option<String>,
    /// The game this projection is for, `None` when there is no game.
    #[serde(default)]
    pub game_id: Option<String>,
    /// Who produced the projection, e.g. `"rotowire"`.
    #[serde(default)]
    pub company: Option<String>,
    /// The name and position Sleeper embeds alongside the numbers. Worth
    /// keeping: it names a player without the 14 MB dictionary.
    #[serde(default)]
    pub player: Option<ProjectedPlayer>,
}

impl Projection {
    /// This projection scored under a league's own rules: every stat key
    /// multiplied by that league's points per unit, summed.
    ///
    /// This is the reason to keep `stats` as a map. A league that gives a
    /// point per reception and one that gives none disagree by 5 points a week
    /// on the same receiver, and the prebaked `pts_half_ppr` is only right for
    /// the leagues that happen to score half-ppr. Keys the league does not
    /// score contribute nothing.
    pub fn points_with(&self, scoring_settings: &HashMap<String, f64>) -> f32 {
        self.stats
            .iter()
            .filter_map(|(key, value)| Some(value * scoring_settings.get(key)?))
            .sum::<f64>() as f32
    }

    /// The prebaked half-ppr projection, when there is one. Only a small
    /// fraction of the entries at a position have it — its absence is how to
    /// tell a real projection from a filler row.
    pub fn half_ppr(&self) -> Option<f32> {
        self.stats.get("pts_half_ppr").map(|points| *points as f32)
    }

    /// Whether anybody actually projected this player. A filler entry carries
    /// only an average draft position.
    pub fn is_projected(&self) -> bool {
        self.stats.keys().any(|key| key.starts_with("pts_"))
    }
}

/// The player object embedded in a [`Projection`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectedPlayer {
    /// Given name.
    #[serde(default)]
    pub first_name: Option<String>,
    /// Family name.
    #[serde(default)]
    pub last_name: Option<String>,
    /// Their listed position.
    #[serde(default)]
    pub position: Option<String>,
    /// Their nfl team, `None` for a free agent.
    #[serde(default)]
    pub team: Option<String>,
    /// `"Questionable"`, `"Out"` and so on; `None` when healthy.
    #[serde(default)]
    pub injury_status: Option<String>,
    /// Seasons played. `0` for a rookie.
    #[serde(default)]
    pub years_exp: Option<i32>,
}

/// One entry of `GET /v1/players/nfl`, trimmed from Sleeper's 53 fields to the
/// ones a scoreboard renders.
///
/// The dropped fields are cross-provider ids and biography; keeping them would
/// mean holding a 14 MB payload in memory and caching it to disk for no gain.
///
/// Nearly everything is optional because team defenses are not shaped like
/// players: a `DEF` entry has **nine** keys where a person has fifty-three,
/// and the rest are absent rather than null. In particular there is no
/// `full_name` on a defense — see [`Player::name`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    /// Digits for a person, a team abbreviation for a defense.
    pub player_id: PlayerId,
    /// Given name; `"Buffalo"` for the Bills defense.
    #[serde(default)]
    pub first_name: Option<String>,
    /// Family name; `"Bills"` for the Bills defense.
    #[serde(default)]
    pub last_name: Option<String>,
    /// Their listed position, including `"DEF"`.
    #[serde(default)]
    pub position: Option<String>,
    /// Their nfl team. `None` for a free agent, and a large share of the
    /// dictionary is free agents, since it holds every player Sleeper has ever
    /// had a record for.
    #[serde(default)]
    pub team: Option<String>,
    /// The positions they are eligible at, which is what a FLEX slot cares
    /// about; a player can be eligible somewhere they are not listed.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub fantasy_positions: Vec<String>,
    /// `"Questionable"`, `"Doubtful"`, `"Out"`, `"IR"`, `"PUP"` and others;
    /// `None` when healthy. Kept as a string rather than an enum because
    /// Sleeper has added values over time and an exhaustive enum would fail to
    /// parse the next one.
    #[serde(default)]
    pub injury_status: Option<String>,
    /// Free text, and not always one word: `"Knee - Meniscus"`.
    #[serde(default)]
    pub injury_body_part: Option<String>,
    /// Roster status: `"Active"`, `"Inactive"`, `"Practice Squad"` and so on.
    #[serde(default)]
    pub status: Option<String>,
    /// Sleeper's `full_name`, which **defenses do not have**. Prefer
    /// [`Player::name`], which fills it in.
    #[serde(default)]
    pub full_name: Option<String>,
}

impl Player {
    /// The player's full name, built from the parts when Sleeper did not send
    /// one — which is every team defense.
    pub fn name(&self) -> String {
        if let Some(full) = self
            .full_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            return full.to_owned();
        }
        let first = self.first_name.as_deref().unwrap_or("").trim();
        let last = self.last_name.as_deref().unwrap_or("").trim();
        match (first.is_empty(), last.is_empty()) {
            (false, false) => format!("{first} {last}"),
            (true, false) => last.to_owned(),
            (false, true) => first.to_owned(),
            // Nothing to build a name from, so show the id rather than a blank
            // row: an unnamed player is a bug worth seeing, not hiding.
            (true, true) => self.player_id.to_string(),
        }
    }

    /// The name at menu bar width: `"J. Allen"`, or `"Bills D/ST"` for a
    /// defense, whose two names are a city and a nickname rather than a
    /// person's.
    pub fn short_name(&self) -> String {
        let last = self.last_name.as_deref().unwrap_or("").trim();
        if self.is_defense() {
            return if last.is_empty() {
                format!("{} D/ST", self.player_id)
            } else {
                format!("{last} D/ST")
            };
        }
        let initial = self
            .first_name
            .as_deref()
            .unwrap_or("")
            .trim()
            .chars()
            .next();
        match (initial, last.is_empty()) {
            (Some(initial), false) => format!("{initial}. {last}"),
            (None, false) => last.to_owned(),
            _ => self.name(),
        }
    }

    /// Whether this is a team defense rather than a person.
    pub fn is_defense(&self) -> bool {
        self.position.as_deref() == Some("DEF")
            || self.fantasy_positions.iter().any(|slot| slot == "DEF")
    }

    /// Whether the player carries an injury designation of any kind.
    pub fn is_injured(&self) -> bool {
        self.injury_status
            .as_deref()
            .is_some_and(|status| !status.trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = include_str!("../tests/fixtures/nfl_state.json");
    const USER: &str = include_str!("../tests/fixtures/user.json");
    const LEAGUES: &str = include_str!("../tests/fixtures/leagues.json");
    const LEAGUE: &str = include_str!("../tests/fixtures/league.json");
    const LEAGUE_USERS: &str = include_str!("../tests/fixtures/league_users.json");
    const ROSTERS: &str = include_str!("../tests/fixtures/rosters.json");
    const MATCHUPS: &str = include_str!("../tests/fixtures/matchups_week2.json");
    const PROJECTIONS: &str = include_str!("../tests/fixtures/projections_qb.json");

    fn rosters() -> Vec<Roster> {
        serde_json::from_str(ROSTERS).expect("rosters fixture")
    }

    fn matchups() -> Vec<Matchup> {
        serde_json::from_str(MATCHUPS).expect("matchups fixture")
    }

    #[test]
    fn state_carries_the_week_everything_hangs_off() {
        let state: State = serde_json::from_str(STATE).expect("state fixture");
        assert_eq!(state.week, 2);
        assert_eq!(state.season, "2026");
        assert_eq!(state.season_type, "regular");
        assert_eq!(state.display_week, Some(2));
        assert_eq!(state.leg, Some(2));
        assert_eq!(state.season_year(), Some(2026));
    }

    #[test]
    fn user_keeps_the_id_and_expands_the_avatar_hash() {
        let user: User = serde_json::from_str(USER).expect("user fixture");
        assert_eq!(user.user_id.as_str(), "100000000000000001");
        assert_eq!(user.display_name, "manager_a");
        assert_eq!(
            user.avatar_url().as_deref(),
            Some("https://sleepercdn.com/avatars/11111111111111111111111111111111")
        );
    }

    /// A username that does not exist comes back as HTTP 200 with a body of
    /// `null`, so the decode has to survive it.
    #[test]
    fn a_null_body_decodes_as_a_missing_user() {
        let user: Option<User> = serde_json::from_str("null").expect("null body");
        assert!(user.is_none());
    }

    #[test]
    fn the_leagues_list_has_two_fields_the_single_league_fetch_does_not() {
        let leagues: Vec<League> = serde_json::from_str(LEAGUES).expect("leagues fixture");
        assert_eq!(leagues.len(), 3);

        assert_eq!(leagues[0].display_order, Some(0));
        // A league with no transactions yet sends null here, not zero.
        assert_eq!(leagues[0].last_transaction_id, None);
        assert_eq!(leagues[1].last_transaction_id, Some(5000000000000000002));

        let league: League = serde_json::from_str(LEAGUE).expect("league fixture");
        assert_eq!(league.display_order, None);
        assert_eq!(league.last_transaction_id, None);
    }

    /// The reason `scoring_settings` and `settings` are maps: one account's
    /// three leagues disagree on how many keys they have.
    #[test]
    fn scoring_settings_are_a_different_shape_in_every_league() {
        let leagues: Vec<League> = serde_json::from_str(LEAGUES).expect("leagues fixture");
        let mut sizes: Vec<usize> = leagues
            .iter()
            .map(|league| league.scoring_settings.len())
            .collect();
        sizes.sort_unstable();
        assert_eq!(sizes, vec![42, 44, 128]);
    }

    #[test]
    fn league_keeps_the_lineup_template_and_the_settings_that_matter() {
        let league: League = serde_json::from_str(LEAGUE).expect("league fixture");
        assert_eq!(league.league_id.as_str(), "200000000000000001");
        assert_eq!(league.total_rosters, 12);
        assert_eq!(league.status, "in_season");
        assert_eq!(league.roster_positions.len(), 27);
        assert_eq!(league.roster_positions[0], "QB");
        assert_eq!(league.roster_positions[11], "SUPER_FLEX");
        assert_eq!(league.scoring_settings.get("rec"), Some(&1.0));
        assert_eq!(league.scoring_settings.get("pass_yd"), Some(&0.04));
        // Week 2 is live while only week 1 has been scored.
        assert_eq!(league.settings.last_scored_leg, Some(1));
        assert_eq!(league.settings.leg, Some(2));
        assert_eq!(league.settings.playoff_week_start, Some(15));
        assert_eq!(league.settings.num_teams, Some(12));
    }

    #[test]
    fn league_users_fall_back_to_the_display_name_without_a_team_name() {
        let users: Vec<LeagueUser> =
            serde_json::from_str(LEAGUE_USERS).expect("league users fixture");
        assert_eq!(users.len(), 6);

        // No team_name key at all in this member's metadata.
        assert_eq!(users[0].metadata.team_name, None);
        assert_eq!(users[0].team_name(), "manager_a");

        assert_eq!(users[1].team_name(), "Team C");
    }

    /// The two avatar conventions: a bare hash needs the cdn prefix, a
    /// metadata avatar is already a url and must be left alone.
    #[test]
    fn a_metadata_avatar_url_wins_over_the_account_hash() {
        let users: Vec<LeagueUser> =
            serde_json::from_str(LEAGUE_USERS).expect("league users fixture");
        assert_eq!(
            users[0].avatar_url().as_deref(),
            Some("https://sleepercdn.com/avatars/11111111111111111111111111111111")
        );
        assert_eq!(
            users[3].avatar_url().as_deref(),
            Some("https://example.invalid/avatars/manager_h.png")
        );
    }

    #[test]
    fn roster_points_are_two_integers_put_back_together() {
        let rosters = rosters();
        assert_eq!(rosters.len(), 6);

        let first = &rosters[0];
        assert_eq!(first.roster_id, RosterId(1));
        assert_eq!(first.settings.fpts, 83);
        assert_eq!(first.settings.fpts_decimal, 10);
        assert!((first.settings.points() - 83.10).abs() < 0.001);
        assert!((first.settings.points_against() - 176.52).abs() < 0.001);
        assert!((first.settings.potential_points() - 90.10).abs() < 0.001);
        assert_eq!(first.settings.record(), "0-1");
        assert_eq!(first.players.len(), 19);
    }

    /// `"0"` is not a player id. It is an empty lineup slot and it is never in
    /// `players`, so anything that looks it up has to check first.
    #[test]
    fn a_starters_list_can_hold_an_empty_slot() {
        let rosters = rosters();
        let first = &rosters[0];
        assert_eq!(first.starters.len(), 12);
        assert!(first.starters[3].is_empty_slot());
        assert!(!first.players.contains(&first.starters[3]));
        assert!(!first.starters[0].is_empty_slot());
    }

    /// A roster with no ir or taxi players sends `null` rather than `[]`.
    #[test]
    fn a_null_list_decodes_as_an_empty_one() {
        let roster: Roster = serde_json::from_str(
            r#"{"roster_id":3,"owner_id":null,"players":null,"starters":null}"#,
        )
        .expect("sparse roster");
        assert!(roster.players.is_empty());
        assert!(roster.starters.is_empty());
        assert_eq!(roster.owner_id, None);
        assert_eq!(roster.settings.record(), "0-0");
    }

    #[test]
    fn matchup_points_are_the_sum_of_the_starters() {
        let matchups = matchups();
        assert_eq!(matchups.len(), 6);

        let first = &matchups[0];
        assert_eq!(first.roster_id, RosterId(1));
        assert_eq!(first.matchup_id, Some(4));
        assert_eq!(first.starters.len(), 12);
        assert_eq!(first.starters_points.len(), 12);
        assert_eq!(first.players_points.len(), 20);

        let summed: f32 = first.starters_points.iter().sum();
        assert!((summed - first.score()).abs() < 0.01);
        // No commissioner override in this fixture, so score() is points.
        assert_eq!(first.custom_points, None);
    }

    /// The same week's roster and matchup disagree about slot 3: the roster
    /// still has the empty slot, the matchup has a player in it. The matchup
    /// is the live one, which is why the ui reads lineups from here.
    #[test]
    fn the_matchup_lineup_wins_over_the_roster_lineup() {
        let roster_slot = &rosters()[0].starters[3];
        let matchup_slot = &matchups()[0].starters[3];
        assert!(roster_slot.is_empty_slot());
        assert_eq!(matchup_slot.as_str(), "8148");
    }

    #[test]
    fn starter_points_pair_each_slot_with_its_score() {
        let matchups = matchups();
        let lineup = matchups[0].starter_points();
        assert_eq!(lineup.len(), 12);
        assert_eq!(lineup[3].0.as_str(), "8148");
        assert!((lineup[3].1 - 5.3).abs() < 0.001);
        assert!((lineup[6].1 - 7.0).abs() < 0.001);

        // players_points is the keyed view and agrees with the positional one.
        assert_eq!(matchups[0].player_points(&lineup[3].0), Some(5.3));
        // A player who is not on this roster has no entry at all.
        assert_eq!(matchups[0].player_points(&PlayerId::from("1")), None);
    }

    /// The vectors are not guaranteed to be the same length. A plain zip would
    /// truncate the lineup and shift every slot label after the gap.
    #[test]
    fn starter_points_survives_mismatched_lengths() {
        let mut matchup = matchups().remove(0);

        matchup.starters_points.truncate(4);
        let short = matchup.starter_points();
        assert_eq!(short.len(), 12, "every slot still gets a row");
        assert!((short[3].1 - 5.3).abs() < 0.001);
        assert_eq!(short[11].1, 0.0, "a slot with no score reads as zero");

        matchup.starters_points = vec![1.0; 20];
        let long = matchup.starter_points();
        assert_eq!(long.len(), 12, "scores past the last slot are dropped");

        matchup.starters.clear();
        assert!(matchup.starter_points().is_empty());
    }

    #[test]
    fn matchups_group_into_head_to_head_pairs() {
        let matchups = matchups();
        let pairs = pairs(&matchups);
        assert_eq!(pairs.len(), 3);

        let ids: Vec<Option<u32>> = pairs.iter().map(|pair| pair.matchup_id).collect();
        assert_eq!(ids, vec![Some(4), Some(3), Some(1)]);

        let first = &pairs[0];
        assert_eq!(first.home.roster_id, RosterId(1));
        assert_eq!(first.away.map(|away| away.roster_id), Some(RosterId(8)));
        assert!(first.contains(RosterId(8)));
        assert!(!first.contains(RosterId(2)));
        // 24.2 against 105.76: the home team is being beaten badly.
        assert!(first.margin().expect("a margin") < 0.0);
    }

    #[test]
    fn pair_for_roster_puts_the_roster_asked_for_on_the_home_side() {
        let matchups = matchups();

        let pair = pair_for_roster(&matchups, RosterId(8)).expect("roster 8 plays somebody");
        assert_eq!(pair.home.roster_id, RosterId(8));
        assert_eq!(pair.away.map(|away| away.roster_id), Some(RosterId(1)));
        assert!(pair.margin().expect("a margin") > 0.0);
        assert_eq!(
            pair.opponent_of(RosterId(8)).map(|other| other.roster_id),
            Some(RosterId(1))
        );

        assert!(pair_for_roster(&matchups, RosterId(99)).is_none());
    }

    /// An odd team out and a bye are both normal. Neither may drop a roster.
    #[test]
    fn an_unpaired_roster_still_comes_back() {
        let mut matchups = matchups();
        matchups.truncate(5);
        let odd = pairs(&matchups);
        assert_eq!(odd.len(), 3, "five rosters still make three rows");
        let lonely = odd
            .iter()
            .find(|pair| pair.away.is_none())
            .expect("the odd roster out is still here");
        assert_eq!(lonely.home.roster_id, RosterId(2));
        assert_eq!(lonely.margin(), None);
        assert!(lonely.opponent_of(lonely.home.roster_id).is_none());

        // Two rosters with no matchup_id are each on a bye, not playing each
        // other.
        for matchup in &mut matchups {
            matchup.matchup_id = None;
        }
        let byes = pairs(&matchups);
        assert_eq!(byes.len(), 5);
        assert!(byes.iter().all(|pair| pair.away.is_none()));
    }

    #[test]
    fn projections_decode_both_the_full_and_the_sparse_shape() {
        let projections: Vec<Projection> =
            serde_json::from_str(PROJECTIONS).expect("projections fixture");
        assert_eq!(projections.len(), 6);

        let first = &projections[0];
        assert_eq!(first.player_id.as_str(), "4984");
        assert_eq!(first.week, 2);
        assert_eq!(first.season, "2026");
        assert_eq!(first.opponent.as_deref(), Some("DET"));
        assert_eq!(first.team.as_deref(), Some("BUF"));
        assert_eq!(first.company.as_deref(), Some("rotowire"));
        assert_eq!(first.half_ppr(), Some(22.02));
        assert!(first.is_projected());
        assert_eq!(
            first.player.as_ref().and_then(|p| p.last_name.as_deref()),
            Some("Allen")
        );

        // The filler entry: no game, no team, and a stats map of one key. It
        // also omits four top-level fields the real ones carry.
        let last = &projections[5];
        assert_eq!(last.stats.len(), 1);
        assert_eq!(last.team, None);
        assert_eq!(last.opponent, None);
        assert_eq!(last.game_id, None);
        assert_eq!(last.half_ppr(), None);
        assert!(!last.is_projected());
    }

    /// The whole reason `stats` and `scoring_settings` are both maps: the same
    /// projection is worth different points in different leagues.
    #[test]
    fn a_projection_scores_against_the_league_that_asked() {
        let projections: Vec<Projection> =
            serde_json::from_str(PROJECTIONS).expect("projections fixture");
        let league: League = serde_json::from_str(LEAGUE).expect("league fixture");
        let allen = &projections[0];

        let scored = allen.points_with(&league.scoring_settings);
        // pass_yd 234.25 * 0.04 + pass_td 1.67 * 4 + rush_yd 28.08 * 0.1 + ...
        assert!(
            (scored - 22.02).abs() < 1.5,
            "league scoring should land near the prebaked half-ppr, got {scored}"
        );

        // A league that scores nothing at all scores this at nothing.
        assert_eq!(allen.points_with(&HashMap::new()), 0.0);

        // Doubling passing touchdowns has to move the number.
        let mut generous = league.scoring_settings.clone();
        generous.insert("pass_td".into(), 8.0);
        assert!(allen.points_with(&generous) > scored);
    }

    #[test]
    fn ids_of_different_kinds_are_different_types() {
        let player = PlayerId::from("4984");
        assert_eq!(player.to_string(), "4984");
        assert_eq!(PlayerId::from(String::from("4984")), player);
        assert_eq!(RosterId::from(1u32).to_string(), "1");
        assert_eq!(Position::DEF.as_str(), "DEF");
        assert_eq!(Position::ALL.len(), 6);
    }
}
