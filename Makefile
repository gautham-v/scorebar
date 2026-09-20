.PHONY: test check fmt clean

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

# The app crate adds run/install/bundle here once it exists:
#
# run: bundle
# 	@pkill -x scorebar 2>/dev/null || true
# 	open "$(TARGET_DIR)/Scorebar.app"
#
# install: bundle
# 	copies Scorebar.app to /Applications and launches it from there, because
# 	launch-at-login registers whatever path the app ran from.
#
# bundle:
# 	./scripts/bundle.sh
