# the sleeper read api

everything scorebar needs is on sleeper's public read api. no key, no oauth, no
cookie: every endpoint below is a plain `GET` against `https://api.sleeper.app`
that answers the same for anyone. that is the whole reason the app can be a menu
bar item with no account setup — the user types a username, and from there it is
all public ids.

sleeper documents part of this surface at `docs.sleeper.com`. the projections
endpoint is not in those docs; it is what the web app itself calls, and it is
marked as such below. sleeper's own guidance is to stay under roughly 1000 calls
per minute, which scorebar is nowhere near — the cache headers matter far more
than the rate limit does, so each section gives the `s-maxage` the cdn actually
returned.

the fixtures in `crates/sleeper/tests/fixtures/` are captured from a real league
and then anonymized: user ids, league ids, display names, team names and avatar
hashes are synthetic. player ids, nfl team abbreviations, stat keys and scoring
keys are real, because those are public reference data and the tests are worth
less if they are made up. every field the live payload has is kept, nulls
included, so serde round-trips exercise the real shape. long arrays are trimmed.

---

## `GET /v1/state/nfl`

documented. the clock everything else hangs off.

```
{"week":2,"season_type":"regular","season":"2026","leg":2,
 "league_season":"2026","previous_season":"2025","season_start_date":"2026-09-09",
 "display_week":2,"league_create_season":"2026","season_has_scores":true}
```

fields that matter: `season` (a string, not a number — it is the path segment in
the leagues call), `week` and `display_week`, and `season_type`.

`week` and `display_week` are not the same field. `week` is the scoring week;
`display_week` is what sleeper's ui labels the current week. they agree
mid-season and diverge at the edges — right after a week's games finish,
`display_week` rolls forward while `week` still points at the week being scored.
scorebar fetches matchups for `week`, because that is the week that has points.

`leg` duplicates `week` for the regular season and is the field leagues use
internally (`league.settings.leg` matches it).

cadence: `s-maxage=60`. fetch it once at launch and then every few minutes; it
only changes at a week boundary.

fixture: `nfl_state.json`.

---

## `GET /v1/user/<username_or_user_id>`

documented. the same path takes either a username or a numeric user id, and
returns the same object for both.

this is the only place the user types anything. scorebar takes a username,
resolves it once, and stores the `user_id` — usernames can be changed, ids
cannot.

**gotcha, and it is the sharp one: an unknown username returns HTTP 200 with a
body of literal `null`, not a 404.** a `reqwest` call that only checks
`status().is_success()` will hand serde the four bytes `null` and get a confusing
deserialize error instead of "no such user". deserialize into
`Option<SleeperUser>` and treat `None` as not-found.

most of the object is dead weight for a read client — `email`, `phone`, `token`,
`cookies`, `currencies`, `notifications`, `summoner_name`, `summoner_region` are
all `null` on a public fetch and always will be. the fields worth keeping are
`user_id`, `username`, `display_name`, `avatar`, `is_bot`.

`avatar` is a bare 32-char hash, not a url. the image lives at
`https://sleepercdn.com/avatars/<hash>` with a smaller copy at
`https://sleepercdn.com/avatars/thumbs/<hash>`. it can be `null`.

cadence: once, at setup. cache the id.

fixture: `user.json`.

---

## `GET /v1/user/<user_id>/leagues/nfl/<season>`

documented. every league that user is in for that season, as an array.

this is what populates the league picker. the season segment is a string year;
passing a season the user has no leagues in returns `[]`, not an error — that is
the normal answer in the offseason before rollover, and it is also what you get
if you pass a `user_id` that does not exist, so it cannot be used to detect a bad
id.

fields that matter per league: `league_id`, `name`, `status` (`pre_draft`,
`drafting`, `in_season`, `complete`), `total_rosters`, `season`, `sport`,
`avatar`, `roster_positions`, `previous_league_id`.

**gotcha: this list form has two fields the single-league endpoint below does not
— `display_order` and `last_transaction_id`.** every other key is identical. if
one rust struct serves both calls, those two have to be `Option` with
`#[serde(default)]`, or the single-league fetch fails to deserialize.

`display_order` is the user's own ordering of their leagues in the sleeper app,
which is a reasonable default order for the picker.

cadence: once at setup, then on demand. leagues do not appear mid-week.

fixture: `leagues.json` — three leagues, deliberately with different scoring and
settings key sets (see the gotcha under `/v1/league/<id>`).

---

## `GET /v1/league/<league_id>`

documented. one league's configuration.

returns `null` with HTTP 404 for an unknown id — unlike the user endpoint, this
one does use the status code. still worth deserializing into `Option` for
symmetry.

fields that matter:

- `roster_positions` — the ordered lineup slots, e.g.
  `["QB","RB","RB","WR","WR","WR","TE","TE","FLEX","FLEX","FLEX","SUPER_FLEX","BN",...]`.
  this is the single most important field in the whole api for a scoring ui,
  because it is what gives the numbers in `starters_points` their labels. the
  leading entries up to the first `BN` are the starting lineup, in order.
- `scoring_settings` — a flat map of stat key to multiplier.
- `settings` — a flat map of league option to integer. `playoff_week_start`,
  `playoff_teams`, `num_teams`, `leg`, `last_scored_leg` are the interesting
  ones. `last_scored_leg` says which week has final scores, which is how you tell
  a week that is done from a week that is live.
- `status`, `total_rosters`, `previous_league_id`, `draft_id`.

**gotcha: `scoring_settings` and `settings` are not fixed-shape.** across the
three leagues in `leagues.json` the scoring map has 42, 44 and 128 keys, and the
settings map has 50, 51 and 54. a league with idp, bonus tiers or divisions
carries keys a standard league has never heard of. model both as
`HashMap<String, f64>` and `HashMap<String, i64>`. do not write a struct.

`settings` values are integers even where they are conceptually booleans
(`best_ball: 0`) or decimals-as-hundredths. `scoring_settings` values are floats
(`pass_yd: 0.04`).

`metadata` is a `HashMap<String, String>` of odds and ends — `auto_continue`,
`keeper_deadline`, `latest_league_winner_roster_id`, sometimes
`copy_from_league_id`. it can be `null`.

the `last_message_*` and `last_author_*` fields are chat state. they change
constantly and are of no use here; skip them, but leave them out with
`#[serde(default)]` rather than denying unknown fields, since sleeper adds keys.

cadence: once per session, or per league switch. league config does not change
mid-week.

fixture: `league.json`.

---

## `GET /v1/league/<league_id>/users`

documented. the league's members.

this is where display names and team names come from, and it is the only source
for them — rosters carry an `owner_id` and nothing else, so scorebar joins users
to rosters on `owner_id == user_id`.

fields that matter: `user_id`, `display_name`, `avatar`, `is_owner`,
`metadata.team_name`, `metadata.avatar`.

**gotcha: the team name lives in `metadata.team_name`, and it is usually
absent.** in a twelve-team league, three had one. when it is missing the label to
show is `display_name`. `metadata` is a string map whose keys vary per user —
observed: `allow_pn`, `allow_sms`, `mention_pn`, `archived`, `show_mascots`,
`team_name`, `avatar`. `metadata` itself can be `null`.

**second gotcha: `metadata.avatar`, when present, is a full url**, whereas the
top-level `avatar` is a bare hash that needs the sleepercdn prefix. the two are
different things and the metadata one wins when set.

`settings` is `null` on every user in this response. `is_owner` marks the
commissioner and was true for exactly one member in each of the three leagues
observed, but nothing in the payload guarantees that, so treat it as a flag
rather than a key.

cadence: `s-maxage=300`. fetch on league load and then rarely. names change
between weeks, not during a game.

fixture: `league_users.json` — six members, covering a user with a team name, a
user with a metadata avatar url, a commissioner, and users with the minimal
metadata map.

---

## `GET /v1/league/<league_id>/rosters`

documented. one entry per team, season-long state.

fields that matter: `roster_id` (an integer, 1..=n — this is the key matchups
join on), `owner_id`, `players`, `starters`, `settings.wins`/`losses`/`ties`,
and the points pair below.

**gotcha: points are split across two integer fields.** `settings.fpts` is the
whole part and `settings.fpts_decimal` is the hundredths, so `fpts: 83` and
`fpts_decimal: 10` means 83.10, not 83.1 and not 8310. the same holds for
`fpts_against`/`fpts_against_decimal` and `ppts`/`ppts_decimal` (ppts is the
"maximum points possible" number). both halves can be absent when a team has
never scored, so they are `Option<i64>` and the join is
`fpts as f64 + fpts_decimal as f64 / 100.0`.

**gotcha: `starters` can contain the string `"0"`** for an empty lineup slot —
the manager left a slot unfilled. `"0"` is not a player id and is not in
`players`. any code that looks up `starters[i]` in the player map has to skip it.

**gotcha: `starters` here is the *saved* lineup and can disagree with the
lineup in the matchups response for the same week.** the fixtures preserve one
such case: roster 1 has `"0"` at index 3 in `rosters.json` and player `8148` at
index 3 in `matchups_week2.json`. matchups is the live one. for scoring, read
lineups from matchups and use rosters only for season record and ownership.

`reserve` (ir) and `taxi` are subsets of `players` and are often `null` rather
than `[]`. `co_owners`, `keepers`, `player_map` were `null` on every roster
observed, but they are declared, so keep them as `Option`.

`metadata` is again a string map with per-roster keys: `record` (a string of
`W`/`L`/`T` characters, one per week played, e.g. `"WL"`), `streak` (`"1L"`),
notification toggles, and `p_nick_<player_id>` entries where a manager has
nicknamed a player. `record` grows one character per week played (`"W"`, then
`"WW"`) and is absent for a team that has not been scored yet, so the whole
`metadata` map, and every key in it, is optional.

cadence: `s-maxage=300`. once per league load, and after a week flips.

fixture: `rosters.json` — six rosters. player arrays are trimmed to starters,
reserve, taxi and a few bench, so `players` is shorter than a real roster, but
every id referenced by `starters`, `reserve` and `taxi` is present.

---

## `GET /v1/league/<league_id>/matchups/<week>`

documented. **this is the live-scoring endpoint** and the one scorebar polls.

one entry per roster, not per matchup: a twelve-team league returns twelve
objects, and the two teams in a game share a `matchup_id`. the caller pairs them
by grouping on `matchup_id`. sleeper declares `matchup_id` nullable — a roster
with no opponent that week, which happens in playoff byes — so it is
`Option<i64>` even though every regular-season row observed had one.

fields: `roster_id`, `matchup_id`, `points`, `custom_points`, `players`,
`starters`, `starters_points`, `players_points`.

**gotcha, the important one: `starters_points` is positional and parallel to
`starters`, not a map.** index `i` of `starters_points` is the score of the
player at index `i` of `starters`, which is itself parallel to the league's
`roster_positions`. so the slot label for `starters_points[i]` is
`roster_positions[i]`. three arrays, one index. if `starters[i]` is `"0"`, then
`starters_points[i]` is `0.0` and there is no player to name. the two arrays
were always the same length in everything observed, but zipping defensively
costs nothing.

`points` is the team total and equals the sum of `starters_points` exactly
(verified on all twelve rosters in the fixture league). it is a float, and it is
the number the menu bar shows. do not recompute it from `players_points`, which
includes the bench.

`players_points` is a map from player id to score covering every player on the
roster, bench included. its key set matches `players`, so it is the source for a
"your bench outscored your starters" line.

`custom_points` is `null` unless a commissioner has overridden the score. when it
is set it wins over `points`, so the display value is
`custom_points.unwrap_or(points)`.

**gotcha: a week that has not kicked off returns a full, well-formed response
with every number at zero** — `points: 0.0` and a `starters_points` of all
`0.0`, not an empty array and not a 404. "no games yet" and "everyone scored
zero" are indistinguishable from this payload alone; use `state.week` and
`league.settings.last_scored_leg` to tell which one it is.

**gotcha: mid-week, a `0.0` in `starters_points` means one of three unrelated
things** — the player has not played yet, the player is on bye, or the player
played and scored nothing. the endpoint does not distinguish them. resolving it
needs `players_sample.json`-style player data (for the team) plus a schedule,
and scorebar does not currently try.

cadence: `s-maxage=60`. that is the floor: polling faster only hits the cloudflare
cache and returns the same body. 60s during games, much slower otherwise. the
response carries a weak `etag` and honours `If-None-Match` with a real `304`, so
a conditional poll costs almost nothing — worth doing, because this is the only
endpoint scorebar hits on a timer.

fixture: `matchups_week2.json` — six rosters forming three complete head-to-head
pairs (`matchup_id` 1, 3 and 4). trimmed to match `rosters.json`, with
`players_points` filtered to the surviving `players` so the two stay consistent.

---

## `GET /v1/players/nfl`

documented. the full player dictionary: a json object keyed by player id, about
12,200 entries and **14 MB uncompressed**.

this is the only way to turn the player ids in `starters` into names, positions
and teams. there is no per-player endpoint and no way to ask for a subset.

sleeper's docs say to call it at most once a day and they mean it. the cdn
returns `s-maxage=600`, but the underlying data changes on roster-move
timescales, not live ones. scorebar fetches it on first run, writes it to the
cache directory, and refreshes it once a day — never on the scoring path.

fields worth keeping out of the 53: `player_id`, `full_name` (or `first_name` +
`last_name`), `position`, `fantasy_positions`, `team`, `injury_status`,
`injury_body_part`, `status`, `number`, `age`, `years_exp`, `search_rank`. the
rest are cross-provider ids (`espn_id`, `gsis_id`, `sportradar_id`, `yahoo_id`,
`rotowire_id`, `kalshi_id`, `oddsjam_id`, `opta_id`, `pandascore_id`, `swish_id`,
`stats_id`, `fantasy_data_id`) and biography (`birth_city`, `high_school`,
`college`, `height`, `weight`) that a scoring ui has no use for. dropping them
before caching takes the 14 MB down by an order of magnitude.

**gotcha, and this one will break a naive struct: team defenses are not shaped
like players.** the `DEF` entries are keyed by team abbreviation (`"BUF"`, not a
number) and carry **nine** keys where a real player carries fifty-three. the
missing keys are absent, not null — no `full_name`, no `age`, no `search_rank`,
no `metadata`. every field except `player_id`, `position`, `first_name`,
`last_name`, `sport`, `team`, `fantasy_positions`, `active` and `injury_status`
has to be `Option` with `#[serde(default)]`. for a defense, the display name is
`first_name + " " + last_name` ("Buffalo Bills").

**gotcha: `team` is `null` for free agents**, and it is `null` for a large share
of the dictionary, since the file includes every player sleeper has ever had a
record for. `active: false` and `status` values other than `"Active"` are common
for the same reason. filtering to rostered players (the ids actually in the
league's `players` arrays) is the cheap way to avoid caring.

`injury_status` is `null` for healthy players and otherwise one of
`"Questionable"`, `"Doubtful"`, `"Out"`, `"IR"`, `"NA"`, `"Sus"`, `"PUP"`, `"DNR"`,
`"COV"` — a string, and sleeper has added values over time, so parse it as a
string and match on it rather than deriving an exhaustive enum.

`injury_body_part` is free text and not always a single word (`"Knee - Meniscus"`).

`height` and `weight` are strings; `number`, `age`, `years_exp` are numbers.
`metadata` is an optional string map (`channel_id`, `rookie_year`, `genius_id`).

fixture: `players_sample.json` — eight players, not the whole file. a qb, a rb,
an injured wr (`Questionable`), an injured te (`Out`, with a compound body part),
a kicker, a team defense (`"BUF"`, nine keys), a free agent with `team: null`,
and a rookie with `years_exp: 0`. that set covers every optionality above.

---

## `GET /projections/nfl/<season>/<week>`

**not documented.** this path is not on `docs.sleeper.com` and is not under
`/v1/`; it is what sleeper's own web client calls. it has been stable for years
but it carries no compatibility promise, so scorebar treats a failure here as
non-fatal and simply shows no projection.

query parameters, all observed rather than specified:

```
?season_type=regular&position[]=QB&order_by=pts_half_ppr
```

`position[]` repeats for multiple positions. the brackets need percent-encoding
(`position%5B%5D=QB`) in most http clients.

returns an array — 355 objects for QB alone, which is every quarterback on file,
not just the projectable ones.

each object: `player_id`, `week`, `season`, `season_type`, `sport`, `category`
(`"proj"`), `company` (`"rotowire"`), `team`, `opponent`, `game_id`, `date`,
`stats`, and an embedded `player` object with `first_name`, `last_name`,
`position`, `team`, `years_exp` and the injury fields.

`stats` is a flat `HashMap<String, f64>` whose keys match the league's
`scoring_settings` keys, which is what makes it useful: multiplying the two maps
and summing gives a league-accurate projection instead of a generic one. the
prebaked `pts_std`, `pts_ppr` and `pts_half_ppr` are there too when you want a
cheap answer.

**gotcha: `order_by` silently ignores anything it does not recognise.** the value
that actually sorts is the stat key — `order_by=pts_half_ppr`. `ppr_half`,
`half_ppr`, `ppr` and `std` all return HTTP 200 with the array in some arbitrary
order that puts unprojected free agents first. there is no error. if the first
element has no `pts_half_ppr`, the sort did not take.

**gotcha: the key set varies per object.** only 32 of the 355 QB entries had a
`pts_half_ppr` at all; the rest carry a `stats` map of exactly
`{"adp_dd_ppr": 1000.0}`. and the objects with real projections carry four
top-level keys the sparse ones omit entirely — `last_modified`, `status`,
`updated_at`, `week_shard`. all four need `#[serde(default)]`.

**gotcha: `team` and `opponent` are `null` for free agents**, and `date` and
`game_id` are `null` along with them.

cadence: `s-maxage=600`. projections move on news, not on plays. once an hour is
generous; once at launch plus once when the week flips is enough.

fixture: `projections_qb.json` — six entries: four top projected quarterbacks,
one low-scoring but fully populated entry, and one sparse free-agent entry with
`stats: {"adp_dd_ppr": 1000.0}` and the four top-level keys missing.

---

## things that apply everywhere

**not-found is inconsistent.** `/v1/league/<bad id>` gives HTTP 404 with body
`null`. `/v1/user/<bad name>` gives HTTP 200 with body `null`. the leagues list
for a nonexistent user gives HTTP 200 with `[]`. handle all three.

**every endpoint returns a weak etag and honours `If-None-Match` with a real
304.** for the 60-second matchups poll and the 14 MB player file this is the
difference between a well-behaved client and a rude one. store the etag beside
the cached body.

**ids are strings, except when they are not.** `user_id`, `league_id`,
`player_id`, `draft_id` are strings of digits (they are snowflake ids that
overflow a js number, which is why sleeper quotes them). `roster_id` and
`matchup_id` are plain integers. `last_transaction_id` is an unquoted integer
large enough to need `i64`. do not assume one rule.

**sleeper adds fields.** nothing here should use `#[serde(deny_unknown_fields)]`.

**avatars** are bare hashes resolved against `https://sleepercdn.com/avatars/`
(or `/avatars/thumbs/` for the small one), and they can be `null` on users,
leagues and message authors alike.
