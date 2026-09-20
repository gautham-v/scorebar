//! Everything scorebar knows that is not drawing.
//!
//! The menu bar app is a Mac program with an AppKit status item and a gpui
//! popover, and none of that can run in CI or be asserted on. So the part that
//! can — what a week looks like, what it is projected to look like, and how
//! likely you are to win it — lives here, in a crate with no gpui, no AppKit
//! and no platform of its own. It builds and tests anywhere.
//!
//! Three pieces, and they stack:
//!
//! - [`model`](crate::model) is the view model: [`Snapshot`], [`LeagueCard`],
//!   [`Side`], [`WeekState`], and the handful of display helpers that keep the
//!   ui from doing arithmetic. All of it serialisable, because the app writes
//!   the last good snapshot to disk and draws it while the next fetch is in
//!   flight.
//! - [`winprob`](crate::winprob) is the simulation: twenty thousand playouts
//!   of the rest of the week, fitted against Sleeper's own published numbers
//!   and seeded so the answer does not wander between refreshes.
//! - [`snapshot`](crate::snapshot) is the assembly: one function that talks to
//!   the `sleeper` crate, and the small pure ones underneath it that turn
//!   payloads into the model.
//!
//! ```no_run
//! use scorebar_core::snapshot;
//! use sleeper::Sleeper;
//!
//! let api = Sleeper::new()?;
//! let snapshot = snapshot(&api, "some_username")?;
//!
//! // The menu bar shows the game closest to even.
//! if let Some(card) = snapshot.closest_game() {
//!     println!(
//!         "{}  {}  {}%",
//!         card.name,
//!         card.me.score_text(),
//!         card.win_percent()
//!     );
//! }
//! # Ok::<(), scorebar_core::Error>(())
//! ```
//!
//! # What is not here
//!
//! No colours, no sizes, no strings that are really layout. The design is
//! monochrome and shows who is ahead with ink against grey, which is a
//! drawing decision the app's theme owns; this crate hands over numbers and
//! the odd caption and stops there.

mod error;
mod model;
mod snapshot;
mod winprob;

pub use error::{Error, Result};
pub use model::{LeagueCard, Side, Snapshot, WeekState};
pub use snapshot::{
    is_yet_to_play, league_card, league_projections, projected_final, projected_points, sim_side,
    snapshot, week_state, yet_to_play,
};
pub use winprob::{win_probability, SimSide, DEFAULT_SEED, SD_FLOOR, SD_SLOPE, TRIALS};
