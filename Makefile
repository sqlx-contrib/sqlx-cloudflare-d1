# Run these inside the dev shell -- `nix develop`, or a Dev Container, which
# gives you the same toolchain. The commands are bare rather than wrapped in
# `nix develop --command` because that re-evaluates the flake every time, which
# costs a couple of seconds per target and nests a second shell inside the one
# you are probably already in.
#
# The crate only runs on wasm32-unknown-unknown, inside a Worker, but it
# compiles on the host too: that is where the unit tests, clippy and the docs
# run. `make check` covers both targets, because a build that is only ever
# checked on the host says nothing about the one target that matters.
#
# `make test` needs no Worker: tests/driver.rs skips itself unless D1_WORKER_URL
# is set. `make test-worker` builds and serves the test Worker and sets it.

WASM := --target wasm32-unknown-unknown

.PHONY: check
check:
	cargo check --all-targets --all-features
	cargo check --all-features $(WASM)

.PHONY: test
test:
	cargo test --all-targets --all-features
# A second run, because `--all-targets` silently excludes doctests. Unindented,
# so make eats the comment instead of the shell echoing it.
	cargo test --doc --all-features

# The integration tests: builds the test Worker, serves it with
# `wrangler dev --local` on a fresh local D1 and runs every scenario in it. No
# Cloudflare account involved -- see tests/worker/run.sh.
.PHONY: test-worker
test-worker:
	tests/worker/run.sh

# Both targets for clippy too: `cfg(target_arch = "wasm32")` code is invisible
# to a host-only run.
.PHONY: lint
lint:
	cargo fmt --all --check
	cargo clippy --all-targets --all-features
	cargo clippy --all-features $(WASM)

.PHONY: doc
doc:
	cargo doc --no-deps --all-features --open

# What CI runs, and what `doc` deliberately isn't: no browser, and warnings are
# errors -- a broken intra-doc link fails the build instead of waiting to be
# noticed.
.PHONY: doc-check
doc-check:
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
