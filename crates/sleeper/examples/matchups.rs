//! Print this week's matchups for every league a manager is in.
//!
//! The whole of what the menu bar popover needs, in one file, with no account
//! and no key:
//!
//! ```text
//! cargo run -p sleeper --example matchups -- <sleeper username>
//! ```
//!
//! It is also the honest smoke test for the crate — if Sleeper changes a
//! payload out from under us, this stops printing before anything else does.

use std::collections::HashMap;

use sleeper::{pair_for_roster, Result, Sleeper};

fn main() -> Result<()> {
    let Some(username) = std::env::args().nth(1) else {
        eprintln!("usage: matchups <sleeper username>");
        std::process::exit(2);
    };

    let api = Sleeper::new()?;
    let state = api.nfl_state()?;
    let user = api.user(&username)?;
    let season = state.season_year().unwrap_or(0);

    println!(
        "{} — week {} of {}",
        user.display_name, state.week, state.season
    );

    for league in api.leagues(&user.user_id, season)? {
        // Who is who: rosters carry the lineup, league_users the team names.
        let rosters = api.rosters(&league.league_id)?;
        let names: HashMap<_, _> = api
            .league_users(&league.league_id)?
            .into_iter()
            .map(|member| (member.user_id.clone(), member.team_name().to_string()))
            .collect();
        let team_name = |roster: &sleeper::Roster| {
            roster
                .owner_id
                .as_ref()
                .and_then(|owner| names.get(owner))
                .cloned()
                .unwrap_or_else(|| format!("roster {}", roster.roster_id))
        };

        let Some(mine) = rosters
            .iter()
            .find(|roster| roster.owner_id.as_ref() == Some(&user.user_id))
        else {
            continue;
        };

        let matchups = api.matchups(&league.league_id, state.week)?;
        let Some(pair) = pair_for_roster(&matchups, mine.roster_id) else {
            println!("\n{} — no matchup this week", league.name);
            continue;
        };

        println!("\n{}", league.name);
        match pair.away {
            Some(away) => {
                let theirs = rosters
                    .iter()
                    .find(|roster| roster.roster_id == away.roster_id);
                println!(
                    "  {:<28} {:>7.2}",
                    truncate(&team_name(mine), 28),
                    pair.home.score()
                );
                println!(
                    "  {:<28} {:>7.2}",
                    theirs
                        .map(|r| truncate(&team_name(r), 28))
                        .unwrap_or_default(),
                    away.score()
                );
                let margin = pair.margin().unwrap_or_default();
                println!("  {:<28} {:>+7.2}", "margin", margin);
            }
            None => println!(
                "  {:<28} {:>7.2}  (bye)",
                truncate(&team_name(mine), 28),
                pair.home.score()
            ),
        }
    }

    Ok(())
}

/// Keep a team name inside the popover's column width.
fn truncate(name: &str, width: usize) -> String {
    if name.chars().count() <= width {
        return name.to_string();
    }
    name.chars()
        .take(width.saturating_sub(1))
        .collect::<String>()
        + "…"
}
