//! The player dictionary, which is the only way to turn an id into a name.
//!
//! `GET /v1/players/nfl` is a single json object of about 12,200 entries and
//! 14 MB uncompressed, keyed by player id. There is no per-player endpoint and
//! no way to ask for a subset, so an app that wants to print `"J. Allen"` next
//! to `"4984"` has to take the whole thing.
//!
//! Two consequences shaped this module. First, the payload is deserialized
//! straight into [`PlayerIndex`] — a map of the trimmed [`Player`], not of
//! `serde_json::Value` — so the 53 fields per entry are dropped as they are
//! read rather than materialized and then discarded. Second, the index
//! round-trips through serde itself, because the right cadence for this call
//! is once a day: fetch it, write it to the cache directory, and never touch
//! it again on the scoring path.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::{Player, PlayerId};

/// Every player Sleeper knows about, by id.
///
/// Transparent over the map, so a cached index is byte-identical to the shape
/// the api returns and either one deserializes into the other.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlayerIndex {
    players: HashMap<PlayerId, Player>,
}

impl PlayerIndex {
    /// Build an index from an already-decoded map, which is what a test or a
    /// hand-built fixture wants.
    pub fn new(players: HashMap<PlayerId, Player>) -> Self {
        Self { players }
    }

    /// One player, if the dictionary has them.
    pub fn get(&self, player_id: &PlayerId) -> Option<&Player> {
        self.players.get(player_id)
    }

    /// The name to print for an id, at menu bar width: `"J. Allen"`, or
    /// `"Bills D/ST"` for a team defense.
    ///
    /// Returns the raw id for anything not in the dictionary, and a dash for
    /// the `"0"` an empty lineup slot holds. A missing player is a stale
    /// cache, not a crash: Sleeper adds rookies mid-week, and a scoreboard
    /// that panicked on one would be a scoreboard that panicked on Wednesdays.
    pub fn short_name(&self, player_id: &PlayerId) -> String {
        if player_id.is_empty_slot() {
            return "—".to_owned();
        }
        match self.get(player_id) {
            Some(player) => player.short_name(),
            None => player_id.to_string(),
        }
    }

    /// The full name for an id, under the same fallback rules as
    /// [`Self::short_name`].
    pub fn name(&self, player_id: &PlayerId) -> String {
        if player_id.is_empty_slot() {
            return "—".to_owned();
        }
        match self.get(player_id) {
            Some(player) => player.name(),
            None => player_id.to_string(),
        }
    }

    /// How many players the index holds.
    pub fn len(&self) -> usize {
        self.players.len()
    }

    /// Whether the index is empty, which for a fetched one means something
    /// went wrong upstream.
    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }

    /// Every entry, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&PlayerId, &Player)> {
        self.players.iter()
    }

    /// Drop everyone not in `keep`.
    ///
    /// A user's leagues touch a few hundred players out of twelve thousand.
    /// Narrowing the index to the ids actually on a roster before caching it
    /// takes the file down by an order of magnitude, and it is also what makes
    /// `team: null` free agents and retired players stop mattering.
    pub fn retain<I>(&mut self, keep: I)
    where
        I: IntoIterator<Item = PlayerId>,
    {
        let keep: std::collections::HashSet<PlayerId> = keep.into_iter().collect();
        self.players.retain(|player_id, _| keep.contains(player_id));
    }
}

impl From<HashMap<PlayerId, Player>> for PlayerIndex {
    fn from(players: HashMap<PlayerId, Player>) -> Self {
        Self::new(players)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYERS: &str = include_str!("../tests/fixtures/players_sample.json");

    fn index() -> PlayerIndex {
        serde_json::from_str(PLAYERS).expect("players fixture")
    }

    #[test]
    fn the_dump_decodes_into_the_trimmed_struct() {
        let index = index();
        assert_eq!(index.len(), 8);
        assert!(!index.is_empty());

        let allen = index.get(&PlayerId::from("4984")).expect("josh allen");
        assert_eq!(allen.name(), "Josh Allen");
        assert_eq!(allen.short_name(), "J. Allen");
        assert_eq!(allen.position.as_deref(), Some("QB"));
        assert_eq!(allen.team.as_deref(), Some("BUF"));
        assert_eq!(allen.fantasy_positions, vec!["QB".to_string()]);
        assert_eq!(allen.status.as_deref(), Some("Active"));
        assert!(!allen.is_injured());
        assert!(!allen.is_defense());
    }

    /// A defense carries nine keys where a player carries fifty-three, and the
    /// other forty-four are absent rather than null. Every one of them has to
    /// be optional or the whole dump fails to decode.
    #[test]
    fn a_team_defense_has_almost_none_of_a_players_fields() {
        let index = index();
        let bills = index.get(&PlayerId::from("BUF")).expect("bills defense");
        assert_eq!(bills.full_name, None);
        assert_eq!(bills.status, None);
        assert!(bills.is_defense());
        // Built from first_name + last_name, since there is no full_name.
        assert_eq!(bills.name(), "Buffalo Bills");
        assert_eq!(bills.short_name(), "Bills D/ST");
    }

    #[test]
    fn injuries_and_free_agency_survive_the_trim() {
        let index = index();

        let nacua = index.get(&PlayerId::from("9493")).expect("an injured wr");
        assert_eq!(nacua.injury_status.as_deref(), Some("Questionable"));
        assert_eq!(nacua.injury_body_part.as_deref(), Some("Hip"));
        assert!(nacua.is_injured());

        // injury_body_part is free text and not always one word.
        let bowers = index.get(&PlayerId::from("11604")).expect("an injured te");
        assert_eq!(bowers.injury_status.as_deref(), Some("Out"));
        assert_eq!(bowers.injury_body_part.as_deref(), Some("Knee - Meniscus"));

        // A free agent has no team, and null survives as None rather than "".
        let free_agent = index.get(&PlayerId::from("3164")).expect("a free agent");
        assert_eq!(free_agent.team, None);
        assert_eq!(free_agent.injury_status, None);
    }

    #[test]
    fn a_missing_id_falls_back_rather_than_failing() {
        let index = index();
        assert_eq!(index.short_name(&PlayerId::from("4984")), "J. Allen");
        // A rookie signed since the cache was written.
        assert_eq!(index.short_name(&PlayerId::from("99999")), "99999");
        assert_eq!(index.name(&PlayerId::from("99999")), "99999");
        // The "0" an empty lineup slot holds is not a lookup miss.
        assert_eq!(index.short_name(&PlayerId::from("0")), "—");
        assert_eq!(index.name(&PlayerId::from("0")), "—");
    }

    #[test]
    fn retaining_the_rostered_players_shrinks_the_index() {
        let mut index = index();
        index.retain([PlayerId::from("4984"), PlayerId::from("BUF")]);
        assert_eq!(index.len(), 2);
        assert!(index.get(&PlayerId::from("9493")).is_none());
        assert_eq!(index.iter().count(), 2);
    }

    /// The index is cached to disk between runs, so it has to come back out of
    /// json the same shape it went in.
    #[test]
    fn the_index_round_trips_through_json() {
        let index = index();
        let text = serde_json::to_string(&index).expect("serialize");
        let again: PlayerIndex = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(again.len(), index.len());
        assert_eq!(again.short_name(&PlayerId::from("BUF")), "Bills D/ST");
    }
}
