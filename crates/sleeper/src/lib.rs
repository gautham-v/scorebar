//! A client for Sleeper's public read api.
//!
//! Sleeper is a fantasy football platform, and the part of it this crate talks
//! to is entirely public: no api key, no oauth, no cookie. Every endpoint is a
//! plain `GET` against `https://api.sleeper.app` that answers the same for
//! anyone. That is what lets an app built on this ask a new user for a
//! username and nothing else — from there it is all public ids.
//!
//! # Rate budget
//!
//! Sleeper's guidance is to stay under roughly 1000 requests a minute. Nothing
//! built on this crate should come close, and the cdn cache headers are the
//! real constraint anyway: the live endpoints are served with `s-maxage=60`,
//! so polling faster than once a minute re-reads Cloudflare's copy of the same
//! numbers. The player dictionary and the projections sit at `s-maxage=600`
//! and want far longer intervals still — the dictionary is 14 MB and Sleeper
//! asks for once a day.
//!
//! # Shape of the api
//!
//! A week of scores takes four calls: [`Sleeper::nfl_state`] for the week,
//! [`Sleeper::league`] for the scoring rules and the lineup template,
//! [`Sleeper::league_users`] and [`Sleeper::rosters`] for who is who, and then
//! [`Sleeper::matchups`] on a loop. The first four change rarely; only the
//! last one is a poll.
//!
//! ```no_run
//! use sleeper::{pair_for_roster, Sleeper};
//!
//! let api = Sleeper::new()?;
//! let state = api.nfl_state()?;
//! let user = api.user("some_username")?;
//!
//! for league in api.leagues(&user.user_id, 2026)? {
//!     let rosters = api.rosters(&league.league_id)?;
//!     let mine = rosters
//!         .iter()
//!         .find(|roster| roster.owner_id.as_ref() == Some(&user.user_id));
//!     let Some(mine) = mine else { continue };
//!
//!     let matchups = api.matchups(&league.league_id, state.week)?;
//!     if let Some(pair) = pair_for_roster(&matchups, mine.roster_id) {
//!         let them = pair.away.map(|away| away.score()).unwrap_or_default();
//!         println!("{}: {:.2} - {:.2}", league.name, pair.home.score(), them);
//!     }
//! }
//! # Ok::<(), sleeper::Error>(())
//! ```
//!
//! # Offline
//!
//! [`Sleeper::with_base_url`] points the client at any host, so the tests in
//! this crate — and anything built on it — can run against recorded payloads
//! instead of the network. The fixtures under `tests/fixtures/` are captured
//! from a real league with the ids and names replaced.

mod client;
mod error;
mod model;
mod players;

pub use client::{Sleeper, DEFAULT_BASE_URL};
pub use error::{Error, Result};
pub use model::{
    avatar_url, pair_for_roster, pairs, League, LeagueId, LeagueSettings, LeagueUser,
    LeagueUserMetadata, Matchup, Pair, Player, PlayerId, Position, ProjectedPlayer, Projection,
    Roster, RosterId, RosterSettings, State, User, UserId, EMPTY_STARTER,
};
pub use players::PlayerIndex;
