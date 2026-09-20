//! Turning Sleeper's payloads into the thing the ui draws.
//!
//! One public entry point that talks to the network, [`snapshot`], and
//! underneath it a handful of small functions that do not. The split is
//! deliberate: the interesting decisions here — what "projected" means, who
//! has not played yet, whether a week of zeros is a week that has not started
//! — are all arithmetic on payloads, and arithmetic on payloads should be
//! testable by handing it a payload rather than by standing up a server.
//!
//! # What a refresh costs
//!
//! One call for the week, one to resolve the username, one for the league
//! list, one for projections, and then three per league — users, rosters,
//! matchups. The league list already carries `scoring_settings` and
//! `roster_positions`, so there is no per-league configuration fetch on top of
//! that. Projections come back for every position in one call and are scored
//! per league afterwards, because two leagues disagree about what a reception
//! is worth and the prebaked `pts_half_ppr` is only right for the ones that
//! happen to score half-ppr.
//!
//! Projections are the one call allowed to fail quietly. The endpoint is
//! undocumented — it is what Sleeper's own web client calls — so a change
//! there should cost the app its projections and its win probabilities, not
//! its scoreboard.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use sleeper::{
    pair_for_roster, League, LeagueUser, Matchup, PlayerId, Position, Projection, Roster, RosterId,
    Sleeper, UserId,
};

use crate::error::{Error, Result};
use crate::model::{LeagueCard, Side, Snapshot, WeekState};
use crate::winprob::{win_probability, SimSide, DEFAULT_SEED};

/// Fetch everything and assemble one [`Snapshot`].
///
/// Blocking, because the sleeper client is: the app calls this on a background
/// thread and hands the result to the ui thread.
///
/// A league the user has no roster in is skipped rather than failing the
/// refresh — it happens when Sleeper's league list is ahead of its roster
/// list, which is to say on the day a league is created.
pub fn snapshot(api: &Sleeper, username: &str) -> Result<Snapshot> {
    let state = api.nfl_state()?;
    let season = state.season_year().unwrap_or_default();
    let week = state.week;

    let user = api.user(username).map_err(|error| match error {
        sleeper::Error::NotFound { .. } => Error::UnknownUsername {
            username: username.to_owned(),
        },
        other => Error::Sleeper(other),
    })?;

    let leagues = api.leagues(&user.user_id, season)?;
    if leagues.is_empty() {
        return Err(Error::NoLeagues { season });
    }

    // Undocumented endpoint: no projections is a worse scoreboard, not a
    // failed refresh.
    let projections = api
        .projections(season, week, &Position::ALL)
        .unwrap_or_default();

    let mut cards = Vec::with_capacity(leagues.len());
    for league in &leagues {
        let members = api.league_users(&league.league_id)?;
        let rosters = api.rosters(&league.league_id)?;
        let matchups = api.matchups(&league.league_id, week)?;

        if let Some(card) = league_card(
            league,
            week,
            &rosters,
            &members,
            &matchups,
            &user.user_id,
            &projections,
        ) {
            cards.push(card);
        }
    }

    Ok(Snapshot {
        week,
        season,
        fetched_at: now_unix(),
        leagues: cards,
    })
}

/// Build one league's card from payloads that are already in hand.
///
/// `week` is the scoring week from `/v1/state/nfl` rather than the league's
/// own `leg`, which lags it by a few minutes at a week boundary — long enough
/// for a league to call the new week finished before it has started.
///
/// `None` when the user owns no roster in this league, which is the only
/// reason a league would have nothing to say.
pub fn league_card(
    league: &League,
    week: u8,
    rosters: &[Roster],
    members: &[LeagueUser],
    matchups: &[Matchup],
    user_id: &UserId,
    projections: &[Projection],
) -> Option<LeagueCard> {
    let mine = rosters
        .iter()
        .find(|roster| roster.owner_id.as_ref() == Some(user_id))?;

    let names = team_names(members);
    let scored = league_projections(projections, &league.scoring_settings);
    let roster_of =
        |roster_id: RosterId| rosters.iter().find(|roster| roster.roster_id == roster_id);
    let name_of = |roster: Option<&Roster>, roster_id: RosterId| {
        roster
            .and_then(|roster| roster.owner_id.as_ref())
            .and_then(|owner| names.get(owner))
            .cloned()
            .unwrap_or_else(|| format!("Roster {roster_id}"))
    };

    // No pairing at all: the league scheduled this roster nothing this week.
    let Some(pair) = pair_for_roster(matchups, mine.roster_id) else {
        return Some(LeagueCard {
            league_id: league.league_id.clone(),
            name: league.name.clone(),
            me: unplayed_side(mine, name_of(Some(mine), mine.roster_id)),
            opponent: None,
            // Nobody to lose to.
            win_probability: 1.0,
            state: WeekState::NoMatchup,
        });
    };

    let me = side(
        pair.home,
        Some(mine),
        name_of(Some(mine), mine.roster_id),
        &scored,
    );
    let opponent = pair.away.map(|away| {
        let roster = roster_of(away.roster_id);
        side(away, roster, name_of(roster, away.roster_id), &scored)
    });

    let win = match pair.away {
        Some(away) => win_probability(
            &sim_side(pair.home, &scored),
            &sim_side(away, &scored),
            DEFAULT_SEED,
        ),
        None => 1.0,
    };

    Some(LeagueCard {
        league_id: league.league_id.clone(),
        name: league.name.clone(),
        me,
        opponent,
        win_probability: win,
        state: week_state(week, league.settings.last_scored_leg, pair.home, pair.away),
    })
}

/// Score every projection under one league's rules, keyed by player.
///
/// The endpoint returns every player on file at each position, most of them
/// with no projection at all — 355 objects for quarterback alone, of which a
/// couple of dozen are real. The filler rows are dropped here so that nothing
/// downstream has to know they exist.
pub fn league_projections(
    projections: &[Projection],
    scoring_settings: &HashMap<String, f64>,
) -> HashMap<PlayerId, f32> {
    projections
        .iter()
        .filter(|projection| projection.is_projected())
        .map(|projection| {
            (
                projection.player_id.clone(),
                projected_points(projection, scoring_settings),
            )
        })
        .collect()
}

/// One player's projection under one league's scoring rules.
///
/// A league that gives a point per reception and one that gives none disagree
/// by five points a week on the same receiver, so the generic `pts_half_ppr`
/// in the payload is the wrong number for most leagues. Multiplying the
/// projected stat line against the league's own `scoring_settings` is the
/// right one, and [`Projection::points_with`] already does that arithmetic.
///
/// A row nobody projected — they carry an average draft position and nothing
/// else — is zero rather than whatever that stray key multiplies out to.
pub fn projected_points(projection: &Projection, scoring_settings: &HashMap<String, f64>) -> f32 {
    if !projection.is_projected() {
        return 0.0;
    }
    projection.points_with(scoring_settings)
}

/// Whether a starter still has points to come.
///
/// **This is an approximation, and it is the one soft spot in this file.** The
/// matchups payload gives a starter's score and nothing else, so a player who
/// has not kicked off yet and a player who finished his game with zero are
/// byte-identical: both are `0.0`. Telling them apart would need the nfl
/// schedule and a clock, which is a second data source for a distinction that
/// only matters for a few minutes a week.
///
/// The consequence is bounded and the app compensates rather than pretends:
/// a genuine zero is treated as a player yet to play, which inflates the
/// projection and the win probability slightly, and only until the week is
/// scored. [`crate::WeekState`] is what keeps the ui honest in the two cases
/// where this would otherwise read badly — before kickoff, when every starter
/// looks like it is yet to play because it is, and after the final whistle,
/// when the week is marked `Final` and the projection stops being drawn.
///
/// The empty-slot marker is excluded because it is not a player: a manager who
/// left a flex open has nothing coming from it, ever.
pub fn is_yet_to_play(player: &PlayerId, points: f32) -> bool {
    !player.is_empty_slot() && points == 0.0
}

/// How many of a roster's starters have not scored yet. See
/// [`is_yet_to_play`] for what "not scored yet" can and cannot mean.
///
/// Saturates at `u8::MAX`, which no lineup will reach; the cast is narrow
/// because the number is drawn as "5 to play" and a lineup is a dozen slots.
pub fn yet_to_play(matchup: &Matchup) -> u8 {
    matchup
        .starter_points()
        .iter()
        .filter(|(player, points)| is_yet_to_play(player, *points))
        .count()
        .min(u8::MAX as usize) as u8
}

/// A side's projected final: what is on the board plus what the starters still
/// to play are projected for.
///
/// A starter with no entry in `projections` contributes nothing. That is the
/// right answer for the two ways it happens — a player Sleeper never projected
/// (deep bench, practice squad) and a week where the projections call failed —
/// and it means a missing projection understates the finish rather than
/// inventing one.
pub fn projected_final(matchup: &Matchup, projections: &HashMap<PlayerId, f32>) -> f32 {
    matchup
        .starter_points()
        .iter()
        .filter(|(player, points)| is_yet_to_play(player, *points))
        .filter_map(|(player, _)| projections.get(player))
        .fold(matchup.score(), |total, projection| total + projection)
}

/// A matchup reduced to what the simulation needs: the score, and one
/// projection per starter still to play.
///
/// One entry per player rather than a single total, because the spread is
/// per-player and five players summing to 60 are far less volatile than one
/// player projected for 60.
pub fn sim_side(matchup: &Matchup, projections: &HashMap<PlayerId, f32>) -> SimSide {
    SimSide {
        score: matchup.score(),
        remaining: matchup
            .starter_points()
            .iter()
            .filter(|(player, points)| is_yet_to_play(player, *points))
            .filter_map(|(player, _)| projections.get(player).copied())
            .collect(),
    }
}

/// Where a week is, given the league's own clock and the two payloads.
///
/// The order of the checks is the point.
///
/// A bye is decided first, because "there is no opponent" is the most specific
/// thing that can be true about a week and it stays true whether the games
/// have been played or not.
///
/// `Final` comes next, from `last_scored_leg`: the league says which week it
/// has finished scoring, and that is the only trustworthy signal. Everything
/// else about a finished week looks exactly like a week in progress.
///
/// `NotStarted` is last before the default, and it is why it cannot come
/// first: a week of zeros that the league has already scored is a week where
/// both managers were shut out, not a week that has not kicked off.
pub fn week_state(
    week: u8,
    last_scored_leg: Option<u8>,
    me: &Matchup,
    opponent: Option<&Matchup>,
) -> WeekState {
    let Some(opponent) = opponent else {
        return WeekState::Bye;
    };

    if last_scored_leg.is_some_and(|scored| scored >= week) {
        return WeekState::Final;
    }

    if me.score() == 0.0 && opponent.score() == 0.0 {
        return WeekState::NotStarted;
    }

    WeekState::InProgress
}

/// Team name per manager, for joining rosters to the only place names live.
fn team_names(members: &[LeagueUser]) -> HashMap<UserId, String> {
    members
        .iter()
        .map(|member| (member.user_id.clone(), member.team_name().to_owned()))
        .collect()
}

/// One team's [`Side`], from its live matchup and its season roster.
fn side(
    matchup: &Matchup,
    roster: Option<&Roster>,
    team_name: String,
    projections: &HashMap<PlayerId, f32>,
) -> Side {
    Side {
        roster_id: matchup.roster_id,
        team_name,
        record: roster
            .map(|roster| roster.settings.record())
            .unwrap_or_else(|| "0-0".to_owned()),
        score: matchup.score(),
        projected: projected_final(matchup, projections),
        yet_to_play: yet_to_play(matchup),
    }
}

/// A [`Side`] for a roster with no matchup at all: the record is real, and
/// there is no week to report on.
fn unplayed_side(roster: &Roster, team_name: String) -> Side {
    Side {
        roster_id: roster.roster_id,
        team_name,
        record: roster.settings.record(),
        score: 0.0,
        projected: 0.0,
        yet_to_play: 0,
    }
}

/// Seconds since the unix epoch. A clock before 1970 is not worth an error
/// type, so it reads as zero and the ui says the snapshot is very old.
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The week every fixture below is for. The league in [`league`] has
    /// `leg: 2` and `last_scored_leg: 1`, so this is a week in progress.
    const WEEK: u8 = 2;

    /// Payloads are built from json rather than from struct literals, so that
    /// what these tests exercise is the shape Sleeper actually sends —
    /// including the nulls and the missing keys.
    fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> T {
        serde_json::from_str(text).expect("fixture json")
    }

    fn matchup(roster_id: u32, matchup_id: Option<u32>, starters: &[(&str, f32)]) -> Matchup {
        let ids: Vec<&str> = starters.iter().map(|(id, _)| *id).collect();
        let points: Vec<f32> = starters.iter().map(|(_, points)| *points).collect();
        let total: f32 = points.iter().sum();
        from_json(&format!(
            r#"{{"roster_id":{roster_id},"matchup_id":{},"points":{total},
                "starters":{},"starters_points":{},"players":{},"players_points":{{}}}}"#,
            matchup_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "null".to_owned()),
            serde_json::to_string(&ids).expect("ids"),
            serde_json::to_string(&points).expect("points"),
            serde_json::to_string(&ids).expect("ids"),
        ))
    }

    fn league() -> League {
        from_json(
            r#"{"league_id":"100","name":"Test League","season":"2026","total_rosters":2,
                "status":"in_season","roster_positions":["QB","RB","WR","BN"],
                "scoring_settings":{"pass_yd":0.04,"pass_td":4.0,"rec":0.5,"rec_yd":0.1},
                "settings":{"leg":2,"last_scored_leg":1}}"#,
        )
    }

    fn rosters() -> Vec<Roster> {
        from_json(
            r#"[{"roster_id":1,"owner_id":"1","players":["100","200","300"],
                 "starters":["100","200","300"],
                 "settings":{"wins":1,"losses":0,"ties":0,"fpts":110,"fpts_decimal":25}},
                {"roster_id":2,"owner_id":"2","players":["400","500"],
                 "starters":["400","500"],
                 "settings":{"wins":0,"losses":1,"ties":0,"fpts":98,"fpts_decimal":5}}]"#,
        )
    }

    fn members() -> Vec<LeagueUser> {
        from_json(
            r#"[{"user_id":"1","display_name":"manager_one","avatar":null,
                 "metadata":{"team_name":"Team One"}},
                {"user_id":"2","display_name":"manager_two","avatar":null,"metadata":{}}]"#,
        )
    }

    /// One real projection and one filler row, which is the ratio the endpoint
    /// actually returns.
    fn projections() -> Vec<Projection> {
        from_json(
            r#"[{"player_id":"100","week":2,"season":"2026",
                 "stats":{"pass_yd":250.0,"pass_td":2.0,"pts_half_ppr":18.0}},
                {"player_id":"200","week":2,"season":"2026",
                 "stats":{"rec":5.0,"rec_yd":60.0,"pts_half_ppr":11.5}},
                {"player_id":"300","week":2,"season":"2026",
                 "stats":{"rec":4.0,"rec_yd":40.0,"pts_half_ppr":8.0}},
                {"player_id":"400","week":2,"season":"2026",
                 "stats":{"rec":6.0,"rec_yd":70.0,"pts_half_ppr":13.0}},
                {"player_id":"500","week":2,"season":"2026",
                 "stats":{"adp_dd_ppr":1000.0}}]"#,
        )
    }

    fn scored() -> HashMap<PlayerId, f32> {
        league_projections(&projections(), &league().scoring_settings)
    }

    #[test]
    fn a_projection_is_scored_by_the_league_that_owns_it() {
        let league = league();
        let projections = projections();
        // 250 * 0.04 + 2 * 4.0 = 18.0, and pts_half_ppr is not a scoring key
        // so it contributes nothing.
        assert!((projected_points(&projections[0], &league.scoring_settings) - 18.0).abs() < 0.001);
        // 5 * 0.5 + 60 * 0.1 = 8.5, which is not the 11.5 in pts_half_ppr:
        // this league does not score like the prebaked number assumes.
        assert!((projected_points(&projections[1], &league.scoring_settings) - 8.5).abs() < 0.001);
    }

    #[test]
    fn a_row_nobody_projected_is_worth_nothing() {
        let league = league();
        let filler = &projections()[4];
        assert!(!filler.is_projected());
        assert_eq!(projected_points(filler, &league.scoring_settings), 0.0);
    }

    #[test]
    fn the_filler_rows_never_reach_the_map() {
        let scored = scored();
        assert_eq!(scored.len(), 4);
        assert!(!scored.contains_key(&PlayerId::from("500")));
        assert!((scored[&PlayerId::from("100")] - 18.0).abs() < 0.001);
    }

    #[test]
    fn a_starter_on_zero_is_counted_as_still_to_play() {
        let matchup = matchup(1, Some(1), &[("100", 22.4), ("200", 0.0), ("300", 0.0)]);
        assert_eq!(yet_to_play(&matchup), 2);
    }

    /// The empty-slot marker is a slot a manager left open, not a player with
    /// points coming.
    #[test]
    fn an_empty_lineup_slot_is_not_a_player_yet_to_play() {
        let matchup = matchup(1, Some(1), &[("100", 22.4), ("0", 0.0), ("300", 0.0)]);
        assert_eq!(yet_to_play(&matchup), 1);
        assert!(!is_yet_to_play(&PlayerId::from("0"), 0.0));
        assert!(is_yet_to_play(&PlayerId::from("300"), 0.0));
        assert!(!is_yet_to_play(&PlayerId::from("300"), 6.2));
    }

    #[test]
    fn a_projected_final_adds_only_what_is_still_to_come() {
        let scored = scored();
        // 100 has played; 200 and 300 have not, and are worth 8.5 and 6.0
        // under this league's rules — not the 11.5 and 8.0 their prebaked
        // half-ppr numbers claim.
        let matchup = matchup(1, Some(1), &[("100", 22.4), ("200", 0.0), ("300", 0.0)]);
        let projected = projected_final(&matchup, &scored);
        assert!((projected - 36.9).abs() < 0.01, "{projected}");
    }

    #[test]
    fn a_finished_lineup_is_projected_at_exactly_its_score() {
        let scored = scored();
        let matchup = matchup(1, Some(1), &[("100", 22.4), ("200", 9.1), ("300", 4.0)]);
        let projected = projected_final(&matchup, &scored);
        assert!((projected - 35.5).abs() < 0.01, "{projected}");
        assert_eq!(yet_to_play(&matchup), 0);
    }

    /// A projection the app never got — the undocumented endpoint failed, or
    /// Sleeper has no line on the player — understates the finish rather than
    /// inventing one.
    #[test]
    fn a_starter_with_no_projection_adds_nothing() {
        let matchup = matchup(1, Some(1), &[("100", 0.0), ("999", 0.0)]);
        let projected = projected_final(&matchup, &scored());
        assert!((projected - 18.0).abs() < 0.01, "{projected}");
        // Still counted as a player with points to come, because he is one.
        assert_eq!(yet_to_play(&matchup), 2);
    }

    #[test]
    fn the_simulation_gets_one_entry_per_player_not_a_total() {
        let matchup = matchup(1, Some(1), &[("100", 22.4), ("200", 0.0), ("300", 0.0)]);
        let side = sim_side(&matchup, &scored());
        assert!((side.score - 22.4).abs() < 0.01);
        assert_eq!(side.remaining.len(), 2);
        let total: f32 = side.remaining.iter().sum();
        assert!((total - 14.5).abs() < 0.01, "{total}");
    }

    #[test]
    fn a_week_with_no_opponent_is_a_bye() {
        let mine = matchup(1, Some(1), &[("100", 0.0)]);
        assert_eq!(week_state(2, Some(1), &mine, None), WeekState::Bye);
        // Even once the league has scored the week: there was still nobody to
        // play, and that is the more useful thing to say.
        assert_eq!(week_state(2, Some(2), &mine, None), WeekState::Bye);
    }

    #[test]
    fn a_scored_week_is_final() {
        let mine = matchup(1, Some(1), &[("100", 22.4)]);
        let theirs = matchup(2, Some(1), &[("400", 19.1)]);
        assert_eq!(
            week_state(2, Some(2), &mine, Some(&theirs)),
            WeekState::Final
        );
        // A league whose scoring has run past this week — a cached snapshot
        // read back on Tuesday — is still final, not in progress.
        assert_eq!(
            week_state(2, Some(3), &mine, Some(&theirs)),
            WeekState::Final
        );
        // One week short of it is not.
        assert_eq!(
            week_state(2, Some(1), &mine, Some(&theirs)),
            WeekState::InProgress
        );
    }

    #[test]
    fn a_week_of_zeros_has_not_kicked_off() {
        let mine = matchup(1, Some(1), &[("100", 0.0), ("200", 0.0)]);
        let theirs = matchup(2, Some(1), &[("400", 0.0)]);
        assert_eq!(
            week_state(2, Some(1), &mine, Some(&theirs)),
            WeekState::NotStarted
        );
        // And with no last_scored_leg at all, which is what a league in its
        // first week sends.
        assert_eq!(
            week_state(1, None, &mine, Some(&theirs)),
            WeekState::NotStarted
        );
    }

    /// The reason `NotStarted` is checked after `Final`: a week both managers
    /// lost 0-0 is finished, not pending.
    #[test]
    fn a_scored_week_of_zeros_is_final_rather_than_pending() {
        let mine = matchup(1, Some(1), &[("100", 0.0)]);
        let theirs = matchup(2, Some(1), &[("400", 0.0)]);
        assert_eq!(
            week_state(2, Some(2), &mine, Some(&theirs)),
            WeekState::Final
        );
    }

    #[test]
    fn a_live_week_is_in_progress() {
        let mine = matchup(1, Some(1), &[("100", 22.4), ("200", 0.0)]);
        let theirs = matchup(2, Some(1), &[("400", 8.0)]);
        assert_eq!(
            week_state(2, Some(1), &mine, Some(&theirs)),
            WeekState::InProgress
        );
    }

    #[test]
    fn a_card_carries_both_sides_the_right_way_round() {
        let matchups = vec![
            matchup(1, Some(1), &[("100", 22.4), ("200", 0.0), ("300", 0.0)]),
            matchup(2, Some(1), &[("400", 0.0), ("500", 6.0)]),
        ];
        let card = league_card(
            &league(),
            WEEK,
            &rosters(),
            &members(),
            &matchups,
            &UserId::from("1"),
            &projections(),
        )
        .expect("a card");

        assert_eq!(card.name, "Test League");
        assert_eq!(card.me.roster_id, RosterId(1));
        assert_eq!(card.me.team_name, "Team One");
        assert_eq!(card.me.record, "1-0");
        assert_eq!(card.me.score_text(), "22.40");
        assert_eq!(card.me.yet_to_play, 2);

        let opponent = card.opponent.as_ref().expect("an opponent");
        assert_eq!(opponent.roster_id, RosterId(2));
        // No team_name in this member's metadata, so the display name stands
        // in rather than a blank.
        assert_eq!(opponent.team_name, "manager_two");
        assert_eq!(opponent.record, "0-1");
        assert_eq!(opponent.score_text(), "6.00");

        assert_eq!(card.state, WeekState::InProgress);
        assert!((0.0..=1.0).contains(&card.win_probability));
        // Ahead on the board and with more still to come.
        assert!(card.win_probability > 0.5, "{}", card.win_probability);
    }

    #[test]
    fn the_same_payloads_always_give_the_same_percentage() {
        let matchups = vec![
            matchup(1, Some(1), &[("100", 12.0), ("200", 0.0), ("300", 0.0)]),
            matchup(2, Some(1), &[("400", 0.0), ("500", 14.0)]),
        ];
        let card = |()| {
            league_card(
                &league(),
                WEEK,
                &rosters(),
                &members(),
                &matchups,
                &UserId::from("1"),
                &projections(),
            )
            .expect("a card")
            .win_probability
        };
        assert_eq!(card(()), card(()));
    }

    #[test]
    fn a_bye_is_a_certainty_with_nobody_on_the_other_side() {
        let matchups = vec![matchup(1, Some(1), &[("100", 22.4)])];
        let card = league_card(
            &league(),
            WEEK,
            &rosters(),
            &members(),
            &matchups,
            &UserId::from("1"),
            &projections(),
        )
        .expect("a card");

        assert!(card.opponent.is_none());
        assert_eq!(card.state, WeekState::Bye);
        assert_eq!(card.win_probability, 1.0);
    }

    /// A roster with no pairing at all still gets a card, because the league
    /// is still one of the user's leagues and the popover still lists it.
    #[test]
    fn a_roster_the_league_scheduled_nothing_for_still_gets_a_card() {
        let matchups = vec![matchup(2, Some(1), &[("400", 8.0)])];
        let card = league_card(
            &league(),
            WEEK,
            &rosters(),
            &members(),
            &matchups,
            &UserId::from("1"),
            &projections(),
        )
        .expect("a card");

        assert_eq!(card.state, WeekState::NoMatchup);
        assert_eq!(card.me.team_name, "Team One");
        assert_eq!(card.me.record, "1-0");
        assert_eq!(card.me.score, 0.0);
        assert_eq!(card.me.yet_to_play, 0);
        assert!(card.opponent.is_none());
    }

    #[test]
    fn a_league_the_user_has_no_roster_in_has_no_card() {
        let matchups = vec![matchup(1, Some(1), &[("100", 22.4)])];
        assert!(league_card(
            &league(),
            WEEK,
            &rosters(),
            &members(),
            &matchups,
            &UserId::from("999"),
            &projections(),
        )
        .is_none());
    }

    /// The projections call is the one allowed to fail. When it does, the
    /// scoreboard is still right — only the projections and the probability
    /// lose their information.
    #[test]
    fn no_projections_still_produces_a_scoreboard() {
        let matchups = vec![
            matchup(1, Some(1), &[("100", 22.4), ("200", 0.0)]),
            matchup(2, Some(1), &[("400", 6.0)]),
        ];
        let card = league_card(
            &league(),
            WEEK,
            &rosters(),
            &members(),
            &matchups,
            &UserId::from("1"),
            &[],
        )
        .expect("a card");

        assert_eq!(card.me.score_text(), "22.40");
        assert_eq!(card.me.projected_text(), "22.4");
        assert_eq!(card.me.yet_to_play, 1);
        // With nothing projected for either side, this reduces to the scores.
        assert_eq!(card.win_probability, 1.0);
    }

    #[test]
    fn the_clock_is_a_plain_unix_timestamp() {
        // Some time after this file was written and a long way before the
        // 32-bit rollover: enough to catch milliseconds or nanoseconds.
        assert!(now_unix() > 1_700_000_000, "{}", now_unix());
        assert!(now_unix() < 4_000_000_000, "{}", now_unix());
    }

    /// The live fetch. Ignored by default, on the same convention the sleeper
    /// crate uses: CI stays green when Sleeper is slow, down or has changed
    /// something.
    ///
    /// It takes the username from `SCOREBAR_TEST_USERNAME` rather than
    /// carrying one, because a username in a public repository is somebody's
    /// account.
    ///
    /// ```text
    /// SCOREBAR_TEST_USERNAME=<username> cargo test -p scorebar-core -- --ignored
    /// ```
    mod live {
        use super::*;

        #[test]
        #[ignore = "hits api.sleeper.app; run with --ignored"]
        fn a_real_account_assembles_into_a_snapshot() {
            let Ok(username) = std::env::var("SCOREBAR_TEST_USERNAME") else {
                eprintln!("set SCOREBAR_TEST_USERNAME to run this");
                return;
            };

            let api = Sleeper::new().expect("client");
            let snapshot = snapshot(&api, &username).expect("snapshot");

            assert!(snapshot.week <= 22);
            assert!(snapshot.season >= 2020);
            assert!(!snapshot.leagues.is_empty());
            for card in &snapshot.leagues {
                assert!(!card.name.is_empty());
                assert!(!card.me.team_name.is_empty());
                assert!((0.0..=1.0).contains(&card.win_probability));
                assert!(card.me.projected >= card.me.score);
            }
            assert!(snapshot.closest_game().is_some());
        }

        #[test]
        #[ignore = "hits api.sleeper.app; run with --ignored"]
        fn a_username_nobody_has_is_named_as_such() {
            let api = Sleeper::new().expect("client");
            let error = snapshot(&api, "scorebar_no_such_user_00000000").expect_err("no account");
            assert!(matches!(error, Error::UnknownUsername { .. }), "{error:?}");
            assert!(!error.is_transient());
        }
    }
}
