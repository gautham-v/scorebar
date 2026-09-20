# scorebar

A macOS menu bar item for Sleeper fantasy football: the current week's score next to the clock, and
a popover with every league you are in, who is leading, and by how much.

Rust + [GPUI](https://www.gpui.rs/), sibling of
[claudebar](https://github.com/gautham-v/claudebar).

Give it your Sleeper username and it finds your leagues. The menu bar carries the one still in
doubt; the popover carries them all, each block a matchup with your score, theirs, and a meter of
your chance of winning it. Monochrome, one type size, no Dock icon. There is no account to connect
and no API key: Sleeper's read API is public, so a username is all there is.

**No release yet.** The app builds and runs from source — `make run` and `make install` below —
but no `v*` tag has been cut, so there is nothing on the releases page and nothing in the tap.
The workflow and the cask template are in place and go live with the first tag.

Not affiliated with, endorsed by, or connected to Sleeper. It reads Sleeper's public API the same
way a browser does.

## Running it

With a Rust toolchain (1.98 or newer) and Xcode installed:

```sh
make install   # builds Scorebar.app, copies it to /Applications, launches it
```

`make run` builds and launches from the cargo target directory instead, which is the one to use
while working on it. Both kill a running copy first.

Install it properly — into `/Applications` — before turning on launch at login: macOS registers
whatever path the app was launched from, and a copy under `target/` disappears on the next
`make clean`.

## Settings

The **Settings** row in the popover, or `~/.config/scorebar/config.toml`:

| key | default | |
|---|---|---|
| `sleeper_username` | none | your Sleeper username, the one thing scorebar needs; without it the popover asks for it |
| `menu_bar_title` | `closest_game` | what the menu bar item prints beside the glyph: `closest_game`, `record`, or `glyph_only`. The popover carries every league either way |
| `refresh_seconds` | `60` | how often to refetch. Sleeper's cdn caches these endpoints for 60 seconds, so anything faster refetches the same numbers; the popover offers 1, 5 and 15 minutes and a hand-edited file is clamped to 15 seconds |

## Build

```sh
make test    # the workspace test suite, against recorded fixtures
make check   # fmt and clippy, the same gates CI runs
```

The tests never touch the network. The handful that do are `#[ignore]`d and run only on request:

```sh
cargo test --workspace -- --ignored
```

## Install

Once the first release is cut, a Homebrew cask:

```sh
brew install --cask gautham-v/tap/scorebar
```

`.github/workflows/release.yml` and `packaging/scorebar.rb` are the machinery for that, waiting on
the first `v*` tag. Until then, build it from source.

## Layout

| crate | what it is |
|---|---|
| `crates/sleeper` | the Sleeper read client: leagues, rosters, users, matchups, and the NFL state that says which week it is |
| `crates/core` | what a week looks like and what it is projected to look like: snapshots and win probability, plain arithmetic with no AppKit in it |
| `crates/app` | the menu bar app itself: the status item, the popover, settings, and the refresh loop |

`sleeper` and `core` build and test on any platform. Only `app` needs a Mac, which is why the three
are separate crates.

[docs/sleeper-api.md](docs/sleeper-api.md) is the endpoint-by-endpoint notes the client was written
from, including the parts of the API that are not documented upstream.
[docs/releasing.md](docs/releasing.md) is how a release is cut.

## How the data works

Sleeper's read API is public. There is no account to connect, no OAuth, and no API key: a username
is enough to find your leagues, and everything after that is public league data. scorebar keeps no
credentials, because it has none to keep.

Nothing leaves your machine except the requests to `api.sleeper.app`. There is no telemetry, no
analytics, and no third-party service in the path. scorebar writes `~/.config/scorebar/config.toml`
and a cache of what it last fetched in `~/Library/Caches/scorebar`.

The fixtures under `crates/sleeper/tests/fixtures/` are captured from a real league and anonymized:
user ids, league ids, display names, team names and avatar hashes are synthetic. Player ids, NFL
team abbreviations and stat keys are real, because those are public reference data.

## Licence

MIT. See [LICENSE](LICENSE).
