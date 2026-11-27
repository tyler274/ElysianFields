#!/usr/bin/env bash
# Builds and launches all three live Elysian Fields servers locally on ephemeral
# localhost ports backed by a temporary WAL/journal directory:
#   - ef-clinic-server  (--web-dir web/elysianfields)
#   - ef-mobile-server  (isolated WAL + independent privacy receipt journal)
#   - ef-public-site    (public pages + Dei recovery finder)
#
# Usage:
#   scripts/run_local_stack.sh [--smoke] [--no-build] [--release]
#
# Options:
#   --smoke      Run automated live health, /readyz, /elysianfields/, and /dei
#                crisis-lines checks against all 3 servers, then exit cleanly.
#   --no-build   Skip `cargo build` and reuse existing target binaries.
#   --release    Build and run release binaries instead of debug binaries.
#   -h, --help   Show this usage message.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

smoke=0
no_build=0
profile="debug"
cargo_profile_args=()

usage() {
  sed -n '2,16p' "$0"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --smoke)
      smoke=1
      ;;
    --no-build)
      no_build=1
      ;;
    --release)
      profile="release"
      cargo_profile_args=(--release)
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "run_local_stack.sh: unknown argument '$1'" >&2
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
    echo "run_local_stack.sh: python3 not found on PATH and nix is unavailable" >&2
    return 1
  fi
}

if [ "$no_build" -eq 0 ]; then
  echo "==> Building ef-clinic-server, ef-mobile-server, and ef-public-site (${profile})..."
  cargo build --locked "${cargo_profile_args[@]}" \
    -p ef-clinic-server \
    -p ef-mobile-server \
    -p ef-public-site
fi

bin_dir="${root}/target/${profile}"
clinic_bin="${bin_dir}/ef-clinic-server"
mobile_bin="${bin_dir}/ef-mobile-server"
public_bin="${bin_dir}/ef-public-site"

for bin_path in "$clinic_bin" "$mobile_bin" "$public_bin"; do
  if [ ! -x "$bin_path" ]; then
    echo "run_local_stack.sh: missing executable binary: $bin_path" >&2
    exit 1
  fi
done

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/ef-local-stack.XXXXXX")"
clinic_pid=""
mobile_pid=""
public_pid=""

cleanup() {
  local exit_code=$?
  for pid in "$clinic_pid" "$mobile_pid" "$public_pid"; do
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  rm -rf "$tmp_root"
  return "$exit_code"
}
trap cleanup EXIT INT TERM

read -r clinic_port mobile_port public_port <<<"$(run_python - <<'PY'
import socket

socks = []
ports = []
try:
    for _ in range(3):
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.bind(("127.0.0.1", 0))
        ports.append(str(s.getsockname()[1]))
        socks.append(s)
finally:
    for s in socks:
        s.close()
print(" ".join(ports))
PY
)"

mkdir -p "${tmp_root}/clinic-wal" "${tmp_root}/mobile-wal" "${tmp_root}/mobile-journal" "${tmp_root}/logs"

EF_CLINIC_BIND="127.0.0.1:${clinic_port}" \
EF_CLINIC_DATA_DIR="${tmp_root}/clinic-wal" \
  "$clinic_bin" --web-dir "${root}/web/elysianfields" \
  >"${tmp_root}/logs/clinic.log" 2>&1 &
clinic_pid=$!

EF_MOBILE_BIND="127.0.0.1:${mobile_port}" \
EF_MOBILE_DATA_DIR="${tmp_root}/mobile-wal" \
EF_MOBILE_JOURNAL_DIR="${tmp_root}/mobile-journal" \
  "$mobile_bin" \
  >"${tmp_root}/logs/mobile.log" 2>&1 &
mobile_pid=$!

EF_PUBLIC_BIND="127.0.0.1:${public_port}" \
EF_PUBLIC_MODE="production" \
  "$public_bin" \
  >"${tmp_root}/logs/public.log" 2>&1 &
public_pid=$!

echo "==> Waiting for all 3 servers to become ready..."
run_python - "$clinic_port" "$mobile_port" "$public_port" "$smoke" <<'PY'
import json
import sys
import time
import urllib.error
import urllib.request

clinic_port, mobile_port, public_port = sys.argv[1], sys.argv[2], sys.argv[3]
smoke_mode = sys.argv[4] == "1"

def fetch(url: str, timeout: float = 2.0) -> tuple[int, dict[str, str], str]:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:
            headers = {k.lower(): v for k, v in resp.headers.items()}
            return resp.status, headers, resp.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as exc:
        headers = {k.lower(): v for k, v in exc.headers.items()}
        body = exc.read().decode("utf-8", errors="replace")
        return exc.code, headers, body

def wait_ok(url: str, label: str, timeout: float = 15.0) -> tuple[int, dict[str, str], str]:
    deadline = time.monotonic() + timeout
    last_err = None
    while time.monotonic() < deadline:
        try:
            status, headers, body = fetch(url)
            if status == 200:
                return status, headers, body
            last_err = f"HTTP {status}"
        except Exception as exc:
            last_err = str(exc)
        time.sleep(0.1)
    raise RuntimeError(f"{label} ({url}) did not return HTTP 200 within {timeout}s: {last_err}")

clinic_url = f"http://127.0.0.1:{clinic_port}/elysianfields/"
mobile_ready_url = f"http://127.0.0.1:{mobile_port}/elysianfields/mobile-api/readyz"
public_health_url = f"http://127.0.0.1:{public_port}/health"
public_ready_url = f"http://127.0.0.1:{public_port}/readyz"

c_status, c_headers, c_body = wait_ok(clinic_url, "ef-clinic-server /elysianfields/")
m_status, m_headers, m_body = wait_ok(mobile_ready_url, "ef-mobile-server /readyz")
p_health_status, _, _ = wait_ok(public_health_url, "ef-public-site /health")
p_ready_status, _, _ = wait_ok(public_ready_url, "ef-public-site /readyz")

if not smoke_mode:
    print(f"ef-clinic-server ready: {clinic_url}")
    print(f"ef-mobile-server ready: {mobile_ready_url}")
    print(f"ef-public-site ready:   http://127.0.0.1:{public_port}/")
    sys.exit(0)

# Detailed smoke verification across all 3 live services.
csp = c_headers.get("content-security-policy", "")
assert "default-src 'self'" in csp, f"missing strict CSP on /elysianfields/: {csp!r}"
assert "/elysianfields" in c_body, "clinic shell HTML missing /elysianfields mount reference"

trav_status, _, _ = fetch(f"http://127.0.0.1:{clinic_port}/elysianfields/../Cargo.toml")
assert trav_status in (400, 404), f"expected 400/404 on traversal, got {trav_status}"

dei_status, _, dei_body = wait_ok(f"http://127.0.0.1:{public_port}/dei", "ef-public-site /dei")
idx_988 = dei_body.find("988")
idx_911 = dei_body.find("911")
idx_samhsa = dei_body.find("SAMHSA")
facility_markers = [
    'id="facilities"',
    'id="state-directory"',
    'data-facility-id="',
    'class="facility',
    "Listed Facilities",
]
listing_indices = [dei_body.find(m) for m in facility_markers if dei_body.find(m) >= 0]
assert idx_988 >= 0 and idx_911 >= 0 and idx_samhsa >= 0, "missing 988/911/SAMHSA crisis lines on /dei"
assert listing_indices, "missing facility listing marker on /dei"
first_listing = min(listing_indices)
assert max(idx_988, idx_911, idx_samhsa) < first_listing, (
    f"crisis lines ({idx_988}, {idx_911}, {idx_samhsa}) must appear before facility listings ({first_listing})"
)

invite_status, invite_headers, invite_body = wait_ok(
    f"http://127.0.0.1:{public_port}/invite", "ef-public-site /invite"
)
robots_ok = (
    ("noindex" in invite_body.lower() and "nofollow" in invite_body.lower())
    or ("noindex" in invite_headers.get("x-robots-tag", "").lower())
)
assert robots_ok, "expected noindex/nofollow on /invite"

grafana_status, grafana_headers, _ = fetch(f"http://127.0.0.1:{public_port}/grafana/")
assert grafana_status == 404, f"expected 404 on /grafana/, got {grafana_status}"
assert "no-store" in grafana_headers.get("cache-control", "").lower(), "expected no-store on /grafana/"

summary = {
    "status": "passed",
    "clinic": {
        "url": clinic_url,
        "status": c_status,
        "strict_csp": True,
        "traversal_rejected_status": trav_status,
    },
    "mobile": {
        "readyz_url": mobile_ready_url,
        "status": m_status,
        "body": m_body.strip(),
    },
    "public_site": {
        "health_status": p_health_status,
        "readyz_status": p_ready_status,
        "dei_status": dei_status,
        "crisis_lines_before_facilities": True,
        "invite_status": invite_status,
        "invite_noindex": robots_ok,
        "grafana_status": grafana_status,
    },
}
print(json.dumps(summary, indent=2))
PY

if [ "$smoke" -eq 1 ]; then
  echo "==> Local 3-server smoke check PASSED; tearing down servers."
  exit 0
fi

echo "==> All 3 servers running (Ctrl-C to stop)."
wait
