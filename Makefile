.PHONY: run install bundle test check fmt clean

# Honour CARGO_TARGET_DIR / .cargo/config.toml rather than assuming ./target.
TARGET_DIR := $(shell cargo metadata --no-deps --format-version 1 | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')

# Build the bundle and launch it (kills any running copy first).
run: bundle
	@pkill -x scorebar 2>/dev/null || true
	open "$(TARGET_DIR)/Scorebar.app"

# Put the app somewhere permanent and run it from there. Launch-at-login
# registers whatever path the app was launched from, so a copy that lives in
# /Applications is the one worth registering.
install: bundle
	@pkill -x scorebar 2>/dev/null || true
	rm -rf "/Applications/Scorebar.app"
	cp -R "$(TARGET_DIR)/Scorebar.app" "/Applications/Scorebar.app"
	open "/Applications/Scorebar.app"

bundle:
	./scripts/bundle.sh

test:
	cargo test --workspace

# The same two gates CI runs, so a green `make check` means a green CI.
check:
	cargo fmt --all --check
	cargo clippy --all-targets --workspace -- -D warnings

fmt:
	cargo fmt --all

clean:
	cargo clean
