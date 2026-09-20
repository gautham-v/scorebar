//! The view model: exactly what the popover and the menu bar item draw.
//!
//! Nothing in here fetches, caches or decides anything. It is the shape the ui
//! reads, and the rule it is built to is that the ui should never have to do
//! arithmetic — if a number has to be rounded, joined or compared before it
//! can be drawn, that belongs here, once, rather than in three render
//! functions that drift apart.
//!
//! Everything is `Serialize + Deserialize` because the app writes the last
//! good [`Snapshot`] to disk. A menu bar item that showed nothing for the two
//! seconds of its first fetch would flicker on every launch; instead it draws
//! yesterday's scoreboard immediately and replaces it when the fetch lands.
//! That is also why [`Snapshot::fetched_at`] exists — a restored snapshot has
//! to be able to say how old it is.
//!
//! One thing that is deliberately *not* here: colour. The design shows who is
//! ahead with ink against secondary grey, and which value gets the ink is a
//! drawing decision the theme owns. This module hands over the numbers.

use serde::{Deserialize, Serialize};
use sleeper::{LeagueId, RosterId};

/// Everything the app knows about one moment, across every league.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The scoring week these matchups are from.
    pub week: u8,
    /// The season, as a number rather than the string Sleeper sends.
    pub season: u16,
    /// When this was fetched, as plain seconds since the unix epoch.
    ///
    /// Not a `DateTime`: this crate has no clock dependency and no opinion
    /// about time zones, and an `i64` is the one representation that survives
    /// a json round trip through any of them unchanged. The app formats it.
    pub fetched_at: i64,
    /// One card per league, in the order the leagues came back — which is the
    /// order the user dragged them into in Sleeper's own app.
    pub leagues: Vec<LeagueCard>,
}

impl Snapshot {
    /// The league whose game is closest to even, which is the one the menu bar
    /// shows by default.
    ///
    /// "Closest" is by win probability rather than by margin, because a
    /// four-point lead with the whole afternoon left is not the same game as a
    /// four-point lead with nobody left to play, and only one of those is
    /// worth glancing at.
    ///
    /// Ties keep the earlier league: [`Self::leagues`] is already in the
    /// user's own order, so the stable answer is the one further up their
    /// list. `None` when there are no leagues at all.
    pub fn closest_game(&self) -> Option<&LeagueCard> {
        self.leagues.iter().min_by(|a, b| {
            let a = (a.win_probability - 0.5).abs();
            let b = (b.win_probability - 0.5).abs();
            a.total_cmp(&b)
        })
    }

    /// The card for one league, if this snapshot has it. The app looks a
    /// league up by id when the user has pinned one to the menu bar.
    pub fn league(&self, league_id: &LeagueId) -> Option<&LeagueCard> {
        self.leagues
            .iter()
            .find(|card| &card.league_id == league_id)
    }
}

/// One league's week: the two teams, and how the game is going.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeagueCard {
    /// Sleeper's id for the league, which is how the app remembers a choice
    /// the user made about it.
    pub league_id: LeagueId,
    /// The league's name, as its members named it.
    pub name: String,
    /// The user's own team.
    pub me: Side,
    /// The other team. `None` on a bye and in any week the league is not
    /// pairing teams; see [`Self::state`] for which.
    pub opponent: Option<Side>,
    /// The chance [`Self::me`] finishes ahead, in `0.0..=1.0`.
    ///
    /// A bye or an unpaired week is `1.0`: there is nobody to lose to.
    pub win_probability: f32,
    /// Where the week is, which is what a score of zero needs to be readable.
    pub state: WeekState,
}

impl LeagueCard {
    /// The win probability as the whole number the ui prints: `63` for `0.63`.
    ///
    /// Clamped as well as rounded, so a value that arrived out of range from a
    /// stale cache prints as `0` or `100` rather than as nonsense.
    pub fn win_percent(&self) -> u32 {
        (self.win_probability * 100.0).round().clamp(0.0, 100.0) as u32
    }
}

/// One team in one week.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Side {
    /// The roster's id within this league. Unique here, not across leagues.
    pub roster_id: RosterId,
    /// The name to print: the manager's team name when they set one, their
    /// display name when they did not.
    pub team_name: String,
    /// The season record, `"1-0"` or `"1-0-1"`, ready to print.
    pub record: String,
    /// Points on the board this week.
    pub score: f32,
    /// The projected final: [`Self::score`] plus what the starters still to
    /// play are projected for.
    pub projected: f32,
    /// How many starters have not scored yet. An approximation — see
    /// [`crate::yet_to_play`] — which is why it sits next to
    /// [`LeagueCard::state`] rather than being trusted alone.
    pub yet_to_play: u8,
}

impl Side {
    /// The score as a scoreboard prints it: two decimals, always both.
    /// Fantasy games are decided in hundredths and a trailing zero that came
    /// and went would make the column jump.
    pub fn score_text(&self) -> String {
        format!("{:.2}", self.score)
    }

    /// The projected final at one decimal. A projection is not precise enough
    /// to deserve the second one, and the caption it sits in is 11px.
    pub fn projected_text(&self) -> String {
        format!("{:.1}", self.projected)
    }
}

/// Where a week is, which a payload of zeros cannot say on its own.
///
/// Sleeper answers a week that has not kicked off with a complete, well-formed
/// response full of zeros — identical to a week in which everybody scored
/// nothing. This is how the two are told apart, and it is the only reason the
/// ui can caption an empty scoreboard correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeekState {
    /// Games are being played and the numbers are moving.
    InProgress,
    /// Kickoff has not happened. Every score is zero because there is nothing
    /// to score yet.
    NotStarted,
    /// The league has finished scoring this week; the numbers are settled.
    Final,
    /// No opponent this week. Common in playoff brackets with an odd number of
    /// teams, and in the weeks a league sits out.
    Bye,
    /// The league scheduled no game for this roster at all — a league that is
    /// not pairing teams this week, or one that has not started.
    NoMatchup,
}

impl WeekState {
    /// The caption the popover prints under a matchup, or `None` for a week in
    /// progress, where the scores say it better than a word would.
    pub fn caption(&self) -> Option<&'static str> {
        match self {
            WeekState::InProgress => None,
            WeekState::NotStarted => Some("Not started"),
            WeekState::Final => Some("Final"),
            WeekState::Bye => Some("Bye"),
            WeekState::NoMatchup => Some("No matchup"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(score: f32, projected: f32) -> Side {
        Side {
            roster_id: RosterId(1),
            team_name: "Team One".to_owned(),
            record: "1-0".to_owned(),
            score,
            projected,
            yet_to_play: 3,
        }
    }

    fn card(name: &str, win_probability: f32) -> LeagueCard {
        LeagueCard {
            league_id: LeagueId::from(name),
            name: name.to_owned(),
            me: side(56.92, 131.6),
            opponent: Some(side(26.0, 79.0)),
            win_probability,
            state: WeekState::InProgress,
        }
    }

    fn snapshot(cards: Vec<LeagueCard>) -> Snapshot {
        Snapshot {
            week: 2,
            season: 2026,
            fetched_at: 1_757_000_000,
            leagues: cards,
        }
    }

    #[test]
    fn a_score_always_carries_both_decimals() {
        assert_eq!(side(56.92, 0.0).score_text(), "56.92");
        assert_eq!(side(100.0, 0.0).score_text(), "100.00");
        assert_eq!(side(0.0, 0.0).score_text(), "0.00");
        assert_eq!(side(7.1, 0.0).score_text(), "7.10");
    }

    #[test]
    fn a_projection_carries_one() {
        assert_eq!(side(0.0, 131.62).projected_text(), "131.6");
        assert_eq!(side(0.0, 131.66).projected_text(), "131.7");
        assert_eq!(side(0.0, 99.0).projected_text(), "99.0");
    }

    #[test]
    fn a_probability_prints_as_a_whole_percent() {
        assert_eq!(card("a", 0.0).win_percent(), 0);
        assert_eq!(card("a", 0.634).win_percent(), 63);
        assert_eq!(card("a", 0.635).win_percent(), 64);
        assert_eq!(card("a", 1.0).win_percent(), 100);
    }

    /// A snapshot restored from an old cache should not be able to print
    /// "-400%" no matter what is in the file.
    #[test]
    fn an_out_of_range_probability_is_clamped_rather_than_printed() {
        assert_eq!(card("a", -4.0).win_percent(), 0);
        assert_eq!(card("a", 12.0).win_percent(), 100);
    }

    #[test]
    fn the_closest_game_is_the_one_nearest_even() {
        let snapshot = snapshot(vec![
            card("runaway", 0.96),
            card("tight", 0.48),
            card("losing", 0.16),
        ]);
        assert_eq!(snapshot.closest_game().expect("a card").name, "tight");
    }

    /// Nearest to even, not highest and not lowest: a game being lost narrowly
    /// is closer than one being won comfortably.
    #[test]
    fn closest_means_nearest_fifty_from_either_direction() {
        let snapshot = snapshot(vec![card("winning big", 0.80), card("losing close", 0.44)]);
        assert_eq!(
            snapshot.closest_game().expect("a card").name,
            "losing close"
        );
    }

    #[test]
    fn an_exact_tie_keeps_the_users_own_order() {
        let snapshot = snapshot(vec![card("first", 0.40), card("second", 0.60)]);
        assert_eq!(snapshot.closest_game().expect("a card").name, "first");
    }

    #[test]
    fn a_manager_with_no_leagues_has_no_closest_game() {
        assert!(snapshot(Vec::new()).closest_game().is_none());
    }

    #[test]
    fn a_league_can_be_found_by_id() {
        let snapshot = snapshot(vec![card("one", 0.5), card("two", 0.5)]);
        let found = snapshot.league(&LeagueId::from("two")).expect("a card");
        assert_eq!(found.name, "two");
        assert!(snapshot.league(&LeagueId::from("three")).is_none());
    }

    #[test]
    fn a_week_that_needs_a_word_gets_one() {
        assert_eq!(WeekState::InProgress.caption(), None);
        assert_eq!(WeekState::NotStarted.caption(), Some("Not started"));
        assert_eq!(WeekState::Final.caption(), Some("Final"));
        assert_eq!(WeekState::Bye.caption(), Some("Bye"));
        assert_eq!(WeekState::NoMatchup.caption(), Some("No matchup"));
    }

    /// The cache is the whole reason these types are serialisable, so the
    /// round trip is worth an assertion rather than an assumption.
    #[test]
    fn a_snapshot_round_trips_through_json() {
        let mut snapshot = snapshot(vec![card("one", 0.63)]);
        snapshot.leagues[0].opponent = None;
        snapshot.leagues[0].state = WeekState::Bye;

        let text = serde_json::to_string(&snapshot).expect("serialize");
        let again: Snapshot = serde_json::from_str(&text).expect("deserialize");

        assert_eq!(again, snapshot);
        assert_eq!(again.leagues[0].state, WeekState::Bye);
        assert_eq!(again.leagues[0].me.score_text(), "56.92");
    }
}
