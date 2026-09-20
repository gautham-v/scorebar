# scorebar

A macOS menu bar item for Sleeper fantasy football: the current week's score next to the clock, and
a popover with every league you are in, who is leading, and by how much.

**It is not built yet.** What is in this repository today is `crates/sleeper`, the client the app
will read from: the types and the read calls that turn a username into leagues, rosters, users and
weekly matchups. There is no menu bar app, no release, and nothing to install. The app crate is
next, in Rust on [GPUI](https://www.gpui.rs/), sibling of
[claudebar](https://github.com/gautham-v/claudebar).

Not affiliated with, endorsed by, or connected to Sleeper. It reads Sleeper's public API the same
way a browser does.

## Build

With a Rust toolchain (1.98 or newer):

```sh
make test    # the workspace test suite, against recorded fixtures
make check   # fmt and clippy, the same gates CI runs
```

The tests never touch the network. The handful that do are `#[ignore]`d and run only on request:

```sh
cargo test --workspace -- --ignored
```

## Install

Nothing to install yet. Once the app crate exists and the first release is cut, it will be a
Homebrew cask:

```sh
brew install --cask gautham-v/tap/scorebar
```

`.github/workflows/release.yml` and `packaging/scorebar.rb` are the machinery for that, dormant
until the first `v*` tag. The `make run` and `make install` targets arrive with the app crate.

## Layout

| crate | what it is |
|---|---|
| `crates/sleeper` | the Sleeper read client: leagues, rosters, users, matchups, and the NFL state that says which week it is |

The menu bar app crate joins the workspace next and depends on `crates/sleeper`.

[docs/sleeper-api.md](docs/sleeper-api.md) is the endpoint-by-endpoint notes the client was written
from, including the parts of the API that are not documented upstream.

## How the data works

Sleeper's read API is public. There is no account to connect, no OAuth, and no API key: a username
is enough to find your leagues, and everything after that is public league data. scorebar keeps no
credentials, because it has none to keep.

Nothing leaves your machine except the requests to `api.sleeper.app`. There is no telemetry, no
analytics, and no third-party service in the path.

The fixtures under `crates/sleeper/tests/fixtures/` are captured from a real league and anonymized:
user ids, league ids, display names, team names and avatar hashes are synthetic. Player ids, NFL
team abbreviations and stat keys are real, because those are public reference data.

## Licence

MIT. See [LICENSE](LICENSE).
