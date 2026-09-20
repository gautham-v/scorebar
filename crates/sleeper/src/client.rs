//! The http client.
//!
//! Blocking, because the app fetches on a background thread and an async
//! runtime would be a dependency and a lifetime puzzle bought for nothing.
//!
//! Every endpoint here is public and unauthenticated: no key, no oauth, no
//! cookie. Sleeper asks callers to stay under roughly 1000 requests a minute,
//! which this is nowhere near — the cache headers matter far more. The cdn
//! serves the live endpoints with `s-maxage=60`, so polling faster than once a
//! minute just re-reads Cloudflare's copy. The player dictionary and the
//! projections sit at `s-maxage=600` and should be fetched far less often than
//! that: daily for the dictionary, hourly at most for projections.
//!
//! Two of these are not on `docs.sleeper.com`:
//! [`Sleeper::projections`] is the endpoint Sleeper's own web client calls,
//! and it carries no compatibility promise, so a failure there should degrade
//! to showing no projection rather than failing the refresh. And
//! [`Sleeper::players`] is documented but **expensive**: 14 MB, once a day,
//! never on the scoring path.

use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;

use crate::error::{Error, Result};
use crate::model::{League, LeagueId, LeagueUser, Matchup, Position, Projection, Roster, State};
use crate::model::{User, UserId};
use crate::players::PlayerIndex;

/// Where the api lives. Overridable through [`Sleeper::with_base_url`].
pub const DEFAULT_BASE_URL: &str = "https://api.sleeper.app";

/// How long to wait on one request. Generous because of the 14 MB player
/// dictionary; a scoring call that takes anywhere near this has already lost.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to wait for the connection itself, which should be quick or not at
/// all.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A handle on the Sleeper read api.
///
/// Cheap to clone in the sense that matters: the inner reqwest client pools
/// connections, so one instance should be kept for the life of the app rather
/// than built per request.
#[derive(Debug, Clone)]
pub struct Sleeper {
    http: Client,
    base_url: String,
}

impl Sleeper {
    /// A client pointed at the real api.
    pub fn new() -> Result<Self> {
        Self::with_base_url(DEFAULT_BASE_URL)
    }

    /// A client pointed somewhere else — a local server replaying the recorded
    /// fixtures, most usefully. Everything above this line is url building and
    /// decoding, so pointing the base url at a test server is what makes the
    /// whole crate testable without touching the network.
    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self> {
        let http = Client::builder()
            // Sleeper is a small team running a free public api; a request
            // that shows up in their logs should say who it is.
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION"),
                " (+",
                env!("CARGO_PKG_REPOSITORY"),
                ")"
            ))
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            // The player dictionary is 14 MB of json and compresses to about a
            // tenth of that.
            .gzip(true)
            .build()?;

        Ok(Self {
            http,
            // A trailing slash here would produce `//v1/...`, which some
            // servers answer and some do not.
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        })
    }

    /// The base url this client is pointed at.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `GET /v1/state/nfl` — the current week and season.
    ///
    /// Everything else needs this first: the week to ask for matchups, and the
    /// season to ask for leagues.
    pub fn nfl_state(&self) -> Result<State> {
        self.get_required("/v1/state/nfl", &[], || "nfl state".to_owned())
    }

    /// `GET /v1/user/<username_or_id>` — resolve an account.
    ///
    /// Takes either a username or a user id, which is convenient and also the
    /// only reason the app can ask a new user to type one thing. An unknown
    /// username comes back as HTTP **200** with a `null` body rather than a
    /// 404; both become [`Error::NotFound`].
    pub fn user(&self, username_or_id: &str) -> Result<User> {
        let path = format!("/v1/user/{username_or_id}");
        self.get_required(&path, &[], || format!("user \"{username_or_id}\""))
    }

    /// `GET /v1/user/<id>/leagues/nfl/<season>` — every league an account is
    /// in for one season.
    ///
    /// The objects here carry two fields [`Sleeper::league`] does not,
    /// `display_order` and `last_transaction_id`, and are otherwise identical.
    pub fn leagues(&self, user_id: &UserId, season: u16) -> Result<Vec<League>> {
        let path = format!("/v1/user/{user_id}/leagues/nfl/{season}");
        self.get_required(&path, &[], || format!("leagues for user {user_id}"))
    }

    /// `GET /v1/league/<id>` — one league's configuration.
    ///
    /// Worth one call per league per launch: this is where
    /// `scoring_settings`, `roster_positions` and `last_scored_leg` come from,
    /// and none of them change during a week.
    pub fn league(&self, league_id: &LeagueId) -> Result<League> {
        let path = format!("/v1/league/{league_id}");
        self.get_required(&path, &[], || format!("league {league_id}"))
    }

    /// `GET /v1/league/<id>/users` — the league's members.
    ///
    /// The only place team names live. Joins to [`Sleeper::rosters`] on
    /// `user_id` / `owner_id`.
    pub fn league_users(&self, league_id: &LeagueId) -> Result<Vec<LeagueUser>> {
        let path = format!("/v1/league/{league_id}/users");
        self.get_required(&path, &[], || format!("users for league {league_id}"))
    }

    /// `GET /v1/league/<id>/rosters` — season records and roster contents.
    ///
    /// For the current week's lineup use [`Sleeper::matchups`]: the two
    /// disagree during a week and the matchup is the live one.
    pub fn rosters(&self, league_id: &LeagueId) -> Result<Vec<Roster>> {
        let path = format!("/v1/league/{league_id}/rosters");
        self.get_required(&path, &[], || format!("rosters for league {league_id}"))
    }

    /// `GET /v1/league/<id>/matchups/<week>` — the live scoreboard.
    ///
    /// One entry per roster, not per game;
    /// [`crate::model::pairs`] puts the two sides back together.
    ///
    /// This is the poll. `s-maxage=60`, so once a minute is the floor worth
    /// asking for. A week that has not kicked off returns a full payload of
    /// zeros rather than an empty array, so compare the week against
    /// [`crate::model::LeagueSettings::last_scored_leg`] before concluding
    /// that everyone is having a bad Sunday.
    pub fn matchups(&self, league_id: &LeagueId, week: u8) -> Result<Vec<Matchup>> {
        let path = format!("/v1/league/{league_id}/matchups/{week}");
        self.get_required(&path, &[], || {
            format!("matchups for league {league_id} week {week}")
        })
    }

    /// `GET /projections/nfl/<season>/<week>` — **undocumented**.
    ///
    /// Not on `docs.sleeper.com` and not under `/v1/`: this is what Sleeper's
    /// own web client calls. It has been stable for years but promises
    /// nothing, so treat a failure as "no projections today" rather than a
    /// failed refresh.
    ///
    /// Returns every player at the positions asked for, not just the
    /// projected ones — 355 objects for QB alone — and most of them carry a
    /// `stats` map of one key. [`Projection::is_projected`] separates them.
    ///
    /// `order_by` is sent as `pts_half_ppr`, the only value that actually
    /// sorts: the endpoint answers 200 and ignores anything it does not
    /// recognise, with no error to notice.
    pub fn projections(
        &self,
        season: u16,
        week: u8,
        positions: &[Position],
    ) -> Result<Vec<Projection>> {
        let path = format!("/projections/nfl/{season}/{week}");

        let mut query: Vec<(&str, String)> = vec![("season_type", "regular".to_owned())];
        // `position[]` repeats once per position, brackets and all; reqwest
        // percent-encodes them.
        query.extend(
            positions
                .iter()
                .map(|position| ("position[]", position.as_str().to_owned())),
        );
        query.push(("order_by", "pts_half_ppr".to_owned()));

        self.get_required(&path, &query, || {
            format!("projections for {season} week {week}")
        })
    }

    /// `GET /v1/players/nfl` — **expensive**: about 12,200 players and 14 MB
    /// of json.
    ///
    /// The only way to turn a player id into a name. Sleeper asks that this be
    /// called at most once a day and they mean it; cache the result to disk
    /// and keep it off the scoring path.
    ///
    /// Decoded straight out of the response body into the trimmed
    /// [`crate::model::Player`], so the dropped 40-odd fields per entry are
    /// never materialized — the 14 MB is read as a stream rather than held as
    /// a `String` and a tree at the same time.
    pub fn players(&self) -> Result<PlayerIndex> {
        let path = "/v1/players/nfl";
        let Some(response) = self.send(path, &[])? else {
            return Err(Error::NotFound {
                what: "the player dictionary".to_owned(),
            });
        };

        serde_json::from_reader(response).map_err(|source| Error::Decode {
            endpoint: path.to_owned(),
            source,
        })
    }

    /// Send one request. `Ok(None)` is a 404, which every caller turns into
    /// [`Error::NotFound`] with a phrase of its own.
    fn send(&self, path: &str, query: &[(&str, String)]) -> Result<Option<Response>> {
        let response = self
            .http
            .get(format!("{}{path}", self.base_url))
            .query(query)
            .send()?;

        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(Error::Status {
                code: status.as_u16(),
                endpoint: path.to_owned(),
            });
        }
        Ok(Some(response))
    }

    /// Fetch and decode, treating a 404 and a `null` body as the same absence.
    ///
    /// The body is read to a string and then parsed rather than using
    /// reqwest's own json helper, so that a payload Sleeper has changed comes
    /// back as [`Error::Decode`] naming the endpoint instead of an anonymous
    /// reqwest error. The player dictionary is the exception and streams.
    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<Option<T>> {
        let Some(response) = self.send(path, query)? else {
            return Ok(None);
        };
        let body = response.text()?;

        serde_json::from_str::<Option<T>>(&body).map_err(|source| Error::Decode {
            endpoint: path.to_owned(),
            source,
        })
    }

    /// [`Self::get`], with the absence turned into an error. `what` is lazy
    /// because it is only ever used to phrase a message.
    fn get_required<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        what: impl FnOnce() -> String,
    ) -> Result<T> {
        self.get(path, query)?
            .ok_or_else(|| Error::NotFound { what: what() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_slash_does_not_become_a_double_slash() {
        let client = Sleeper::with_base_url("http://127.0.0.1:8080/").expect("client");
        assert_eq!(client.base_url(), "http://127.0.0.1:8080");

        let default = Sleeper::new().expect("client");
        assert_eq!(default.base_url(), DEFAULT_BASE_URL);
    }

    /// Nothing points at api.sleeper.app unless it was asked to, so the test
    /// suite cannot accidentally depend on the network.
    #[test]
    fn a_test_base_url_is_honoured() {
        let client = Sleeper::with_base_url("http://localhost:1").expect("client");
        assert!(client.base_url().starts_with("http://localhost"));
    }

    /// Live checks against the real api. Ignored by default: CI has to stay
    /// green when Sleeper is slow, down, or has changed something. Run them
    /// deliberately with `cargo test -- --ignored`.
    mod live {
        use super::*;

        #[test]
        #[ignore = "hits api.sleeper.app; run with --ignored"]
        fn the_state_endpoint_still_answers_the_shape_we_expect() {
            let client = Sleeper::new().expect("client");
            let state = client.nfl_state().expect("state");
            assert!(state.week <= 22);
            assert!(state.season_year().is_some());
        }

        #[test]
        #[ignore = "hits api.sleeper.app; run with --ignored"]
        fn an_unknown_user_is_not_found_despite_the_200() {
            let client = Sleeper::new().expect("client");
            let error = client
                .user("scorebar_no_such_user_00000000")
                .expect_err("should not resolve");
            assert!(matches!(error, Error::NotFound { .. }), "{error:?}");
        }

        #[test]
        #[ignore = "hits api.sleeper.app; run with --ignored"]
        fn an_unknown_league_is_not_found_via_a_404() {
            let client = Sleeper::new().expect("client");
            let error = client
                .league(&LeagueId::from("000000000000000000"))
                .expect_err("should not resolve");
            assert!(matches!(error, Error::NotFound { .. }), "{error:?}");
        }

        #[test]
        #[ignore = "hits api.sleeper.app and downloads 14 MB; run with --ignored"]
        fn the_player_dictionary_still_decodes() {
            let client = Sleeper::new().expect("client");
            let players = client.players().expect("players");
            assert!(players.len() > 5_000);
            // The shape that breaks naive structs: a defense keyed by team.
            let defense = players
                .get(&crate::model::PlayerId::from("BUF"))
                .expect("a team defense");
            assert!(defense.is_defense());
        }
    }
}
