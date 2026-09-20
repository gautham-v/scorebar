# Releasing

1. Bump `version` in the root `Cargo.toml` under `[workspace.package]`. That one number is what
   the release workflow reads, what the zip is named after, and what the cask installs.
2. Run `make check && make test`, then commit the bump.
3. Tag it and push the tag: `git tag v0.1.0 && git push origin v0.1.0`.

The tag is the only trigger. `.github/workflows/release.yml` then builds `scorebar` for
`aarch64-apple-darwin` and `x86_64-apple-darwin`, `lipo`s them into one binary, runs
`scripts/bundle.sh` to make `Scorebar.app`, zips it to `Scorebar-<version>.zip`, and creates the
GitHub release with the zip and its `.sha256`. A `workflow_dispatch` run does all of that except
publishing, which is the way to check a build without cutting a release.

The `tap` job then rewrites `packaging/scorebar.rb` with the release's version and checksum and
pushes it to `gautham-v/homebrew-tap` as `Casks/scorebar.rb`. Nothing in `packaging/` is edited by
hand; the values checked in are just the previous release's.

## Secrets

| secret | what it is |
|---|---|
| `DEVELOPER_ID_P12` | base64 of the "Developer ID Application" certificate, exported as a .p12 |
| `DEVELOPER_ID_P12_PASSWORD` | the password that .p12 was exported with |
| `APPLE_ID` | the Apple ID that owns the certificate |
| `APPLE_APP_PASSWORD` | an app-specific password for that Apple ID |
| `APPLE_TEAM_ID` | the ten-character team id |
| `TAP_TOKEN` | a fine-grained PAT with `contents: write` on the tap repository only |

Without the five Apple secrets the build still succeeds: the bundle is signed ad hoc, notarization
is skipped, and users get Gatekeeper's "Open Anyway" prompt on first launch. Without `TAP_TOKEN`
the `tap` job fails loudly and the GitHub release still stands — publish the cask by hand, or set
the secret and re-run that job.
