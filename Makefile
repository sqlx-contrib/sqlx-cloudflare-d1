# Run these inside the dev shell -- `nix develop`, or a Dev Container, which
# gives you the same toolchain. The commands are bare rather than wrapped in
# `nix develop --command` because that re-evaluates the flake every time, which
# costs a couple of seconds per target and nests a second shell inside the one
# you are probably already in.
#
# The crates only run on wasm32-unknown-unknown, inside a Worker, but they
# compile on the host too: that is where the unit tests, clippy and the docs
# run. `make check` covers both targets, because a build that is only ever
# checked on the host says nothing about the one target that matters.
#
# `make test` needs no Worker: each driver's tests/driver.rs skips itself
# unless its D1_WORKER_URL or DO_WORKER_URL is set. `make test-worker` builds
# and serves each driver's test Worker in turn and sets it.

WASM := --target wasm32-unknown-unknown

.PHONY: check
check:
	cargo check --workspace --all-targets --all-features
	cargo check --workspace --all-features $(WASM)

.PHONY: test
test:
	cargo test --workspace --all-targets --all-features
# A second run, because `--all-targets` silently excludes doctests. Unindented,
# so make eats the comment instead of the shell echoing it.
	cargo test --workspace --doc --all-features

# The integration tests: builds each driver's test Worker, serves it with
# `wrangler dev --local` on fresh local storage -- a D1 database, a Durable
# Object -- and runs every scenario in it. No Cloudflare account involved --
# see run.sh next to each test Worker.
.PHONY: test-worker
test-worker:
	crates/sqlx-cloudflare-d1/tests/worker/run.sh
	crates/sqlx-cloudflare-do/tests/worker/run.sh

# Both targets for clippy too: `cfg(target_arch = "wasm32")` code is invisible
# to a host-only run.
.PHONY: lint
lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --all-features
	cargo clippy --workspace --all-features $(WASM)

.PHONY: doc
doc:
	cargo doc --workspace --no-deps --all-features --open

# What CI runs, and what `doc` deliberately isn't: no browser, and warnings are
# errors -- a broken intra-doc link fails the build instead of waiting to be
# noticed.
.PHONY: doc-check
doc-check:
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
