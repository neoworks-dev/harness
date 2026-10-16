#!/usr/bin/env bash
# Screenshots the desktop app against a throwaway daemon, headless (svelte-gpui's shot.sh: a
# headless Hyprland output, nothing on the user's screens).
#
#   apps/gui/scripts/shot.sh OUT.png [SECONDS] [host flags…]      e.g. --do press:Send
#
# The daemon runs the scripted echo agent with no services or guests, on a temporary socket and
# data dir, and is stopped afterwards. SEED holds lines to send first, one input each, e.g.
#   SEED=$'hello\nshow me a diff' apps/gui/scripts/shot.sh /tmp/gui.png 4
# Build first: bun apps/gui/build.mjs && cargo build --release -p suite-gui
set -euo pipefail

root=$(cd "$(dirname "$0")/../../.." && pwd)
out=$1
seconds=${2:-3}
shift
if [ $# -gt 0 ]; then shift; fi

tmp=$(mktemp -d)
export SUITE_SOCKET=$tmp/bus.sock SUITE_DATA_DIR=$tmp/data
export SUITE_AGENT=echo SUITE_SERVICES=none SUITE_GUESTS=none
daemon_pid=""
cleanup() {
  kill $daemon_pid 2>/dev/null || true
  rm -rf "$tmp"
}
trap cleanup EXIT

bun run "$root/daemon/src/main.ts" >"$tmp/daemon.log" 2>&1 &
daemon_pid=$!
for _ in $(seq 100); do
  [ -e "$tmp/daemon.json" ] && break
  sleep 0.1
done

mkdir -p "$tmp/work"
cd "$tmp/work"
if [ -n "${SEED:-}" ]; then
  printf '%s\n' "$SEED" | "$root/bin/suite" attach --new >/dev/null
fi

HOST="$root/target/release/suite-gui" \
  "$root/../svelte-gpui/scripts/shot.sh" "$root/apps/gui/dist/gui.js" "$out" "$seconds" "$@"
