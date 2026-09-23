#!/usr/bin/env bash
# Builds the test Worker, serves it with `wrangler dev --local` on a fresh
# local D1, and runs `tests/d1.rs` against it. What `make test-worker` runs.
#
# Fresh means a temporary `--persist-to` directory: the scenarios assume the
# fixture schema and nothing else, and a database left over from the last run
# would be neither.
set -euo pipefail

cd "$(dirname "$0")"
port="${D1_WORKER_PORT:-8787}"
state="$(mktemp -d)"
export WRANGLER_SEND_METRICS=false

worker-build --release
wrangler d1 migrations apply DB --local --persist-to "$state"

wrangler dev --local --persist-to "$state" --port "$port" >"$state/wrangler.log" 2>&1 &
wrangler=$!
trap 'kill "$wrangler" 2>/dev/null || true; rm -rf "$state"' EXIT

for _ in $(seq 1 60); do
  if curl -sf "http://localhost:$port/" >/dev/null; then
    break
  fi
  if ! kill -0 "$wrangler" 2>/dev/null; then
    cat "$state/wrangler.log"
    exit 1
  fi
  sleep 1
done

cd ../..
D1_WORKER_URL="http://localhost:$port" cargo test --test d1 -- --nocapture
