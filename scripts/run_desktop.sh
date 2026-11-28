#!/usr/bin/env bash
# Single-command launcher for the native Bevy desktop apps (`ef-clinic-desktop`
# and `ef-mobile-desktop`), supporting offline demo mode, live local server mode,
# and automated headless smoke verification across both apps.
#
# Usage:
#   scripts/run_desktop.sh clinic [--demo | --with-server] [--locale <bcp47>] [--theme <dark|light>] [--release] [--screenshot <path.png>]
#   scripts/run_desktop.sh mobile [--demo | --with-server] [--locale <bcp47>] [--theme <dark|light>] [--release] [--screenshot <path.png>]
#   scripts/run_desktop.sh --smoke [--release]
#
# Options:
#   --demo                Run the desktop app against its built-in in-process demo state (default).
#   --with-server         Launch the corresponding local headless server on an ephemeral port and connect via HTTP.
#   --locale <bcp47>      Initial UI locale (e.g. en, he, ar, zh-Hans, ja, vi, hi).
#   --theme <dark|light>  Initial WCAG 2.1 AA theme.
#   --screenshot <path>   Capture a PNG screenshot to the given path.
#   --release             Build and run release binaries instead of debug binaries.
#   --smoke               Build and run `--headless-smoke` on both desktop apps when present.
#   -h, --help            Show this usage text.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

usage() {
  sed -n '2,19p' "$0"
}

if [ "$#" -eq 0 ]; then
  usage >&2
  exit 2
fi

target_app=""
smoke=0
with_server=0
profile="debug"
app_args=()

case "$1" in
  clinic|mobile)
    target_app="$1"
    shift
    ;;
  --smoke)
    smoke=1
    shift
    ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    echo "run_desktop.sh: expected 'clinic', 'mobile', or '--smoke', got '$1'" >&2
    exit 2
    ;;
esac

while [ "$#" -gt 0 ]; do
  case "$1" in
    --demo)
      with_server=0
      ;;
    --with-server)
      with_server=1
      ;;
    --release)
      profile="release"
      ;;
    --smoke)
      smoke=1
      ;;
    --locale)
      shift
      locale_val="${1:?--locale requires a BCP 47 tag}"
      app_args+=(--locale "$locale_val")
      ;;
    --locale=*)
      app_args+=(--locale "${1#--locale=}")
      ;;
    --theme)
      shift
      theme_val="${1:?--theme requires dark or light}"
      app_args+=(--theme "$theme_val")
      ;;
    --theme=*)
      app_args+=(--theme "${1#--theme=}")
      ;;
    --screenshot)
      shift
      shot_val="${1:?--screenshot requires a file path}"
      app_args+=(--screenshot "$shot_val")
      ;;
    --screenshot=*)
      app_args+=(--screenshot "${1#--screenshot=}")
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "run_desktop.sh: unknown argument '$1'" >&2
      exit 2
      ;;
  esac
  shift
done

run_python() {
  if command -v python3 >/dev/null 2>&1; then
    python3 "$@"
  elif command -v nix >/dev/null 2>&1; then
    nix shell nixpkgs#python3 -c python3 "$@"
  else
    echo "run_desktop.sh: python3 not found on PATH and nix is unavailable" >&2
    return 1
  fi
}

build_args=(--locked)
if [ "$profile" = "release" ]; then
  build_args+=(--release)
fi
target_dir="${CARGO_TARGET_DIR:-$root/target}/$profile"

if [ "$smoke" -eq 1 ]; then
  crates=()
  if [ -d crates/ef-clinic-desktop ]; then
    crates+=(ef-clinic-desktop)
  fi
  if [ -d crates/ef-mobile-desktop ]; then
    crates+=(ef-mobile-desktop)
  fi
  if [ "${#crates[@]}" -eq 0 ]; then
    echo "run_desktop.sh: no native desktop crates present yet (crates/ef-clinic-desktop or crates/ef-mobile-desktop)."
    exit 0
  fi

  pkg_flags=()
  for crate in "${crates[@]}"; do
    pkg_flags+=(-p "$crate")
  done
  cargo build "${build_args[@]}" "${pkg_flags[@]}"
  for crate in "${crates[@]}"; do
    echo "==> Running ${crate} --headless-smoke"
    "${target_dir}/${crate}" --headless-smoke
  done
  exit 0
fi

desktop_crate="ef-${target_app}-desktop"
server_crate="ef-${target_app}-server"
if [ ! -d "crates/${desktop_crate}" ]; then
  echo "run_desktop.sh: crate directory crates/${desktop_crate} not found" >&2
  exit 1
fi

if [ "$with_server" -eq 0 ]; then
  cargo build "${build_args[@]}" -p "$desktop_crate"
  exec "${target_dir}/${desktop_crate}" --demo "${app_args[@]}"
fi

cargo build "${build_args[@]}" -p "$server_crate" -p "$desktop_crate"

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/elysian-desktop-${target_app}.XXXXXX")
server_pid=""
cleanup() {
  local rc=$?
  set +e
  if [ -n "$server_pid" ] && kill -0 "$server_pid" 2>/dev/null; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$tmp_dir"
  exit "$rc"
}
trap cleanup EXIT INT TERM

server_port=$(run_python - <<'PY'
import socket
with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
    s.bind(("127.0.0.1", 0))
    print(s.getsockname()[1])
PY
)

if [ "$target_app" = "clinic" ]; then
  EF_CLINIC_BIND="127.0.0.1:${server_port}" \
  EF_CLINIC_DATA_DIR="${tmp_dir}/clinic-wal" \
  EF_CLINIC_WEB_DIR="${root}/web/elysianfields" \
    "${target_dir}/${server_crate}" --web-dir "${root}/web/elysianfields" >"${tmp_dir}/server.log" 2>&1 &
  server_pid=$!
  ready_url="http://127.0.0.1:${server_port}/elysianfields/"
else
  EF_MOBILE_BIND="127.0.0.1:${server_port}" \
  EF_MOBILE_DATA_DIR="${tmp_dir}/mobile-wal" \
  EF_MOBILE_JOURNAL_DIR="${tmp_dir}/mobile-journal" \
    "${target_dir}/${server_crate}" >"${tmp_dir}/server.log" 2>&1 &
  server_pid=$!
  ready_url="http://127.0.0.1:${server_port}/elysianfields/mobile-api/readyz"
fi

run_python - "$ready_url" <<'PY'
import sys, time, urllib.error, urllib.request
url = sys.argv[1]
deadline = time.monotonic() + 15.0
last_err = None
while time.monotonic() < deadline:
    try:
        with urllib.request.urlopen(url, timeout=2.0) as resp:
            if resp.status == 200:
                sys.exit(0)
    except Exception as exc:
        last_err = exc
        time.sleep(0.1)
raise SystemExit(f"timed out waiting for local server at {url}: {last_err}")
PY

"${target_dir}/${desktop_crate}" --server "http://127.0.0.1:${server_port}" "${app_args[@]}"
