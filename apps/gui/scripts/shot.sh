#!/usr/bin/env bash
# Screenshots the desktop app against a throwaway daemon, headless (svelte-gpui's shot.sh: a
# headless Hyprland output, nothing on the user's screens).
#
#   apps/gui/scripts/shot.sh OUT.png [SECONDS] [host flags…]      e.g. --do press:Send
#
# The daemon runs the scripted echo agent with no services or guests, on a temporary socket and
# data dir, and is stopped afterwards. SEED holds lines to send first, one input each, e.g.
#   SEED=$'hello\nshow me a diff' apps/gui/scripts/shot.sh /tmp/gui.png 4
# SCENE=neoui SCENE_SCRIPT=scene-neoui.ts SCENE_DOCS=dashboard,form (scripts/neoui-docs.ts) has the
# agent answer with neoui docs, to look at how the window draws them.
# Build first: bun apps/gui/build.mjs && cargo build --release -p neo-gui
set -euo pipefail

root=$(cd "$(dirname "$0")/../../.." && pwd)
out=$1
seconds=${2:-3}
shift
if [ $# -gt 0 ]; then shift; fi

tmp=$(mktemp -d)
export NEO_SOCKET=$tmp/bus.sock NEO_DATA_DIR=$tmp/data
export NEO_AGENT=echo NEO_SERVICES=none NEO_GUESTS=none
daemon_pid=""
cleanup() {
  kill $daemon_pid 2>/dev/null || true
  rm -rf "$tmp"
}
trap cleanup EXIT

# SCENE=side runs a daemon with scripted sessions and events instead (scripts/scene-daemon.ts), in
# a git repository with two changed files, an untracked one and an untracked directory.
if [ -n "${SCENE:-}" ]; then
  mkdir -p "$tmp/work" "$tmp/elsewhere"
  git -C "$tmp/work" init -q
  printf 'one\ntwo\n' >"$tmp/work/a.txt"
  mkdir -p "$tmp/work/src"
  for i in $(seq 1 40); do echo "const value$i = compute($i);"; done >"$tmp/work/src/lib.ts"
  git -C "$tmp/work" add a.txt src/lib.ts
  git -C "$tmp/work" -c user.name=t -c user.email=t@t -c commit.gpgsign=false commit -q -m first
  printf 'one\nthree\n' >"$tmp/work/a.txt"
  printf 'fresh\n' >"$tmp/work/new.txt"
  # SCENE_SETUP is a shell snippet run in the repository, e.g. SCENE_SETUP='git branch side', or
  # "bun run $PWD/apps/gui/scripts/scene-big.ts" for a diff of thousands of lines.
  if [ -n "${SCENE_SETUP:-}" ]; then (cd "$tmp/work" && bash -c "$SCENE_SETUP"); fi
  mkdir -p "$tmp/work/extra"
  printf 'export const one = 1;\n' >"$tmp/work/extra/one.ts"
  sed -i 's/^const value20 = .*/const value20 = computeSomethingMuchLongerThanBefore(20, "a string long enough to wrap in a narrow panel");/; s/^const value37 = .*/const value37 = 37;/' "$tmp/work/src/lib.ts"
  # SCENE_SCRIPT names another script of scripts/: scene-chat.ts is a long conversation.
  SCENE_CWD="$tmp/work" SCENE_ELSEWHERE="$tmp/elsewhere" bun run "$root/apps/gui/scripts/${SCENE_SCRIPT:-scene-daemon.ts}" >"$tmp/daemon.log" 2>&1 &
else
  bun run "$root/daemon/src/main.ts" >"$tmp/daemon.log" 2>&1 &
fi
daemon_pid=$!
ready=$tmp/daemon.json
if [ -n "${SCENE:-}" ]; then ready=$tmp/scene-ready; fi
for _ in $(seq 100); do
  [ -e "$ready" ] && break
  sleep 0.1
done

mkdir -p "$tmp/work"
cd "$tmp/work"
if [ -n "${SEED:-}" ]; then
  printf '%s\n' "$SEED" | "$root/bin/neo" attach --new >/dev/null
fi
# More sessions, each from its own directory (under work/, so the window's own stays the first
# session's): SEED2 in work/site, SEED3 in a worktree of it. A line "wait:60000" keeps one running.
for extra in "SEED2 site" "SEED3 site/.worktrees/dark"; do
  read -r var dir <<<"$extra"
  seed=${!var:-}
  if [ -n "$seed" ]; then
    mkdir -p "$tmp/work/$dir"
    (cd "$tmp/work/$dir" && printf '%s\n' "$seed" | timeout 5 "$root/bin/neo" attach --new >/dev/null) || true
  fi
done

HOST="$root/target/release/neo-gui" \
  "$root/../svelte-gpui/scripts/shot.sh" "$root/apps/gui/dist/gui.js" "$out" "$seconds" "$@"
