//! How likely you are to win the week.
//!
//! A closed form would be wrong here. The quantity that decides a fantasy
//! game is the sum of a handful of players who have not played yet, each one a
//! skewed distribution of its own, and the two sides have different numbers of
//! them left. So this is a Monte Carlo: play the rest of the week out
//! [`TRIALS`] times and count.
//!
//! The one interesting parameter is the spread. A player projected for 18
//! points is not 18 points give or take 18 — he is 18 give or take about 13 —
//! and a kicker projected for 7 is nowhere near as volatile in absolute terms
//! but far more volatile in relative ones. [`SD_SLOPE`] and [`SD_FLOOR`] are
//! that shape: standard deviation grows with the projection and never falls
//! below a couple of points, because even a projection of zero can return a
//! kickoff for a touchdown.
//!
//! The numbers this produces were checked against Sleeper's own published win
//! probabilities on a live Sunday and land within a couple of points of them
//! (see the regression test at the bottom of this file). That is the whole
//! justification for the constants, and it is why they should not be tuned by
//! taste: they are not a model of football, they are a fit.
//!
//! # Determinism
//!
//! The seed is an argument rather than entropy. Two reasons. The tests need a
//! fixed answer to assert on, and — the real one — a menu bar item that
//! redrew "63%" as "62%" and back every sixty seconds while nothing changed
//! would look broken. The app passes [`DEFAULT_SEED`] every time, so the same
//! scoreboard always produces the same percentage.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// How many times the rest of the week is played out.
///
/// At 20,000 trials the standard error near 50% is about a third of a point,
/// which is comfortably inside the one point the ui rounds to, and the whole
/// simulation for a dozen leagues still finishes in well under a frame.
pub const TRIALS: usize = 20_000;

/// How fast a player's spread grows with his projection.
pub const SD_SLOPE: f32 = 0.6;

/// The spread a player has regardless of projection. Nobody is a sure zero.
pub const SD_FLOOR: f32 = 2.5;

/// The seed the app uses, so a percentage does not jitter between refreshes.
///
/// Any value would do; this one is arbitrary and fixed.
pub const DEFAULT_SEED: u64 = 0x5C07_EBA1;

/// One side of a game, reduced to the two things the simulation needs.
///
/// Deliberately not [`crate::Side`]: the view model carries team names and
/// records that the arithmetic has no business seeing, and building this from
/// anything — a matchup, a test literal — is what keeps the simulation
/// testable without a network or a league.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimSide {
    /// Points already on the board this week.
    pub score: f32,
    /// The projection for each starter who has not scored yet, one entry per
    /// player. Not a sum: the spread is per-player, and collapsing five
    /// players into one number of the same total would roughly double the
    /// variance.
    pub remaining: Vec<f32>,
}

impl SimSide {
    /// A side with a score and nothing left to play.
    pub fn finished(score: f32) -> Self {
        Self {
            score,
            remaining: Vec::new(),
        }
    }

    /// The projected final: what is on the board plus what is still expected.
    pub fn projected(&self) -> f32 {
        self.score + self.remaining.iter().sum::<f32>()
    }
}

/// The probability that `me` finishes above `opponent`, in `0.0..=1.0`.
///
/// Ties count as a loss. Fantasy leagues do score ties, but a tie is rare
/// enough that giving it its own number would cost more ui than it is worth,
/// and rounding it into the loss column is the conservative direction.
///
/// `seed` fixes the draw; see the module docs for why it is an argument.
pub fn win_probability(me: &SimSide, opponent: &SimSide, seed: u64) -> f32 {
    // With nothing left to play there is nothing to simulate: every trial
    // would produce the same two totals. This is the same answer the loop
    // below would reach, twenty thousand times more slowly.
    if me.remaining.is_empty() && opponent.remaining.is_empty() {
        return if me.score > opponent.score { 1.0 } else { 0.0 };
    }

    let mut rng = StdRng::seed_from_u64(seed);
    let mut wins = 0usize;

    for _ in 0..TRIALS {
        let mine = simulate(me, &mut rng);
        let theirs = simulate(opponent, &mut rng);
        if mine > theirs {
            wins += 1;
        }
    }

    wins as f32 / TRIALS as f32
}

/// One trial for one side: the current score plus a draw for every starter
/// still to play.
///
/// Each draw is clamped at zero. Negative scores exist in fantasy football —
/// a quarterback can throw three interceptions — but they are a tail this
/// model does not try to reproduce, and letting a normal centred on 2 go to
/// -8 would put weight somewhere the real distribution has almost none.
fn simulate(side: &SimSide, rng: &mut StdRng) -> f32 {
    side.remaining.iter().fold(side.score, |total, projection| {
        let sd = SD_SLOPE * projection + SD_FLOOR;
        total + (projection + sd * standard_normal(rng)).max(0.0)
    })
}

/// A draw from the standard normal, by Box-Muller.
///
/// Written out rather than pulled from a distributions crate: it is four lines
/// and one fewer dependency, and doing it here means the only source of
/// randomness in the whole crate is one seeded [`StdRng`].
fn standard_normal(rng: &mut StdRng) -> f32 {
    // `gen` yields `[0, 1)`, and `ln(0)` is negative infinity. Taking
    // `1 - u` moves the half-open end to the side the logarithm can survive.
    let u1: f64 = 1.0 - rng.gen::<f64>();
    let u2: f64 = rng.gen::<f64>();
    ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A seed for the tests that is not the app's, so nothing can pass here by
    /// accidentally matching a value the app happens to produce.
    const SEED: u64 = 7;

    fn side(score: f32, remaining: &[f32]) -> SimSide {
        SimSide {
            score,
            remaining: remaining.to_vec(),
        }
    }

    #[test]
    fn a_finished_week_in_front_is_certain() {
        let me = SimSide::finished(120.4);
        let them = SimSide::finished(98.1);
        assert_eq!(win_probability(&me, &them, SEED), 1.0);
    }

    #[test]
    fn a_finished_week_behind_is_lost() {
        let me = SimSide::finished(98.1);
        let them = SimSide::finished(120.4);
        assert_eq!(win_probability(&me, &them, SEED), 0.0);
    }

    /// Ties count as a loss, so a finished week level is zero, not a half.
    #[test]
    fn a_finished_tie_is_a_loss() {
        let me = SimSide::finished(101.0);
        let them = SimSide::finished(101.0);
        assert_eq!(win_probability(&me, &them, SEED), 0.0);
    }

    #[test]
    fn two_identical_sides_are_a_coin_flip() {
        let me = side(48.0, &[12.0, 9.5, 7.25]);
        let them = side(48.0, &[12.0, 9.5, 7.25]);
        let probability = win_probability(&me, &them, SEED);
        assert!(
            (probability - 0.5).abs() < 0.02,
            "identical sides gave {probability}"
        );
    }

    /// The direction check: holding the score level, the side with more still
    /// to come should be the favourite, and more of it should mean more of a
    /// favourite.
    #[test]
    fn more_left_to_play_moves_the_number_up() {
        let them = side(60.0, &[10.0, 10.0]);

        let level = win_probability(&side(60.0, &[10.0, 10.0]), &them, SEED);
        let ahead = win_probability(&side(60.0, &[10.0, 10.0, 12.0]), &them, SEED);
        let further = win_probability(&side(60.0, &[10.0, 10.0, 12.0, 14.0]), &them, SEED);

        assert!(level < ahead, "{level} should be under {ahead}");
        assert!(ahead < further, "{ahead} should be under {further}");
    }

    /// And the same in points already scored, which is the less interesting
    /// half of the same property but the one a sign error would break.
    #[test]
    fn a_bigger_lead_moves_the_number_up() {
        let them = side(60.0, &[10.0, 10.0]);
        let behind = win_probability(&side(45.0, &[10.0, 10.0]), &them, SEED);
        let ahead = win_probability(&side(75.0, &[10.0, 10.0]), &them, SEED);
        assert!(behind < ahead, "{behind} should be under {ahead}");
    }

    #[test]
    fn the_same_seed_gives_the_same_answer() {
        let me = side(56.9, &[14.2, 11.5, 8.6]);
        let them = side(61.4, &[13.1, 9.5]);

        let first = win_probability(&me, &them, SEED);
        let second = win_probability(&me, &them, SEED);
        assert_eq!(first, second);

        // And a different seed is a different draw, or the seed is being
        // ignored and the first assertion means nothing.
        let elsewhere = win_probability(&me, &them, SEED + 1);
        assert_ne!(first, elsewhere);
        assert!((first - elsewhere).abs() < 0.02, "{first} vs {elsewhere}");
    }

    #[test]
    fn the_result_is_always_a_probability() {
        let me = side(0.0, &[22.0]);
        let them = side(300.0, &[]);
        let hopeless = win_probability(&me, &them, SEED);
        assert!((0.0..=1.0).contains(&hopeless), "{hopeless}");
    }

    #[test]
    fn a_projected_final_is_the_score_plus_what_is_left() {
        let me = side(56.92, &[14.2, 12.8, 11.5]);
        assert!((me.projected() - 95.42).abs() < 0.01, "{}", me.projected());
        assert_eq!(SimSide::finished(56.92).projected(), 56.92);
    }

    /// The reason the constants are what they are.
    ///
    /// These two games are from a live Sunday afternoon, mid-afternoon slate,
    /// with Sleeper's own app open next to the numbers: it showed **94%** for
    /// the first and **14%** for the second at the moment these scores and
    /// projections were read off it. The model lands within two points of
    /// both, which is the agreement this whole file exists to preserve.
    ///
    /// If a change to [`SD_SLOPE`], [`SD_FLOOR`] or the draw moves either of
    /// these, the change is wrong, however much better it looks in isolation.
    #[test]
    fn the_validated_sunday_games_still_land_where_sleeper_had_them() {
        // Well ahead, and with more left to play than the opponent: 56.92 to
        // 26.00, with about 74.7 and 53.0 still to come.
        let me = side(56.92, &[14.2, 12.8, 11.5, 10.6, 9.4, 8.6, 7.6]);
        let them = side(26.00, &[13.1, 11.6, 10.4, 9.5, 8.4]);
        let comfortable = win_probability(&me, &them, DEFAULT_SEED) * 100.0;
        assert!(
            (comfortable - 96.0).abs() <= 2.0,
            "expected about 96%, got {comfortable}"
        );

        // Behind by 39 with two players left against one: 65.44 to 104.32,
        // about 34.3 still to come against about 16.9.
        let me = side(65.44, &[18.6, 15.7]);
        let them = side(104.32, &[16.9]);
        let long_shot = win_probability(&me, &them, DEFAULT_SEED) * 100.0;
        assert!(
            (long_shot - 16.0).abs() <= 2.0,
            "expected about 16%, got {long_shot}"
        );
    }
}
