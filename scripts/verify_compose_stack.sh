#!/usr/bin/env bash
# Live 5-container integrated Docker Compose + Caddy single-origin gateway E2E
# verification (`postgres:18`, `clinic`, `mobile`, `public-site`, `gateway`).
#
# Usage:
#   scripts/verify_compose_stack.sh [--no-build] [--project <name>]
#     [--clinic-image <tag>] [--mobile-image <tag>] [--public-image <tag>]
#     [--report <path.json>]
#
# Options:
#   --no-build             Reuse already-built local images instead of rebuilding.
#   --project <name>       Docker Compose project name (default: elysian-compose-e2e).
#   --clinic-image <tag>   Tag existing clinic image for Compose reuse under --project.
#   --mobile-image <tag>   Tag existing mobile image for Compose reuse under --project.
#   --public-image <tag>   Tag existing public-site image for Compose reuse under --project.
#   --report <path.json>   Optional path to write structured JSON verification output.
#   -h, --help             Show this usage text.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

no_build=0
project="elysian-compose-e2e"
clinic_image=""
mobile_image=""
public_image=""
report_path=""

usage() {
  sed -n '2,18p' "$0"
}

emit_error() {
  local msg="$1"
  if [ "${GITHUB_ACTIONS:-false}" = "true" ]; then
    echo "::error title=Compose Gateway E2E::${msg}"
  else
    echo "verify_compose_stack.sh ERROR: ${msg}" >&2
  fi
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --no-build)
      no_build=1
      ;;
    --project)
      shift
      project="${1:?--project requires a name}"
      ;;
    --project=*)
      project="${1#--project=}"
      ;;
    --clinic-image)
      shift
      clinic_image="${1:?--clinic-image requires an image tag}"
      ;;
    --clinic-image=*)
      clinic_image="${1#--clinic-image=}"
      ;;
    --mobile-image)
      shift
      mobile_image="${1:?--mobile-image requires an image tag}"
      ;;
    --mobile-image=*)
      mobile_image="${1#--mobile-image=}"
      ;;
    --public-image)
      shift
      public_image="${1:?--public-image requires an image tag}"
      ;;
    --public-image=*)
      public_image="${1#--public-image=}"
      ;;
    --report)
      shift
      report_path="${1:?--report requires a file path}"
      ;;
    --report=*)
      report_path="${1#--report=}"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "verify_compose_stack.sh: unknown argument '$1'" >&2
      exit 2
      ;;
  esac
  shift
done

compose_cmd=(docker compose -p "$project" -f deploy/docker-compose.yml --profile gateway)

cleanup() {
  local rc=$?
  set +e
  if [ "$rc" -ne 0 ]; then
    emit_error "5-container Compose gateway E2E failed (exit ${rc}); collecting diagnostics."
    "${compose_cmd[@]}" ps 2>&1 || true
    "${compose_cmd[@]}" logs --tail=60 2>&1 || true
  fi
  "${compose_cmd[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  exit "$rc"
}
trap cleanup EXIT INT TERM

if [ -n "$clinic_image" ]; then
  docker tag "$clinic_image" "${project}-clinic:latest"
fi
if [ -n "$mobile_image" ]; then
  docker tag "$mobile_image" "${project}-mobile:latest"
fi
if [ -n "$public_image" ]; then
  docker tag "$public_image" "${project}-public-site:latest"
fi

# Start from a clean compose project state.
"${compose_cmd[@]}" down -v --remove-orphans >/dev/null 2>&1 || true

up_args=(up -d --wait --wait-timeout 90)
if [ "$no_build" -eq 1 ]; then
  up_args+=(--no-build)
else
  up_args+=(--build)
fi

echo "==> Starting 5-container Elysian Fields Compose + Caddy gateway stack (project=${project})"
"${compose_cmd[@]}" "${up_args[@]}"

run_python() {
  if command -v python3 >/dev/null 2>&1; then
    python3 "$@"
  elif command -v nix >/dev/null 2>&1; then
    nix shell nixpkgs#python3 -c python3 "$@"
  else
    emit_error "python3 not found on PATH and nix is unavailable"
    return 1
  fi
}

echo "==> Verifying single-origin Caddy gateway (http://127.0.0.1:8080) routes, headers, metrics, and PG18 persistence"
run_python - "$project" "${report_path}" <<'PY'
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

project = sys.argv[1]
report_path = Path(sys.argv[2]) if sys.argv[2] else None
base = "http://127.0.0.1:8080"


def request(
    path: str,
    *,
    method: str = "GET",
    body: dict | None = None,
    timeout: float = 20.0,
) -> tuple[int, dict[str, str], str]:
    deadline = time.monotonic() + timeout
    last_err: Exception | None = None
    data = json.dumps(body).encode("utf-8") if body is not None else None
    headers_in = {"Content-Type": "application/json"} if data is not None else {}
    while time.monotonic() < deadline:
        req = urllib.request.Request(f"{base}{path}", data=data, headers=headers_in, method=method)
        try:
            with urllib.request.urlopen(req, timeout=5.0) as resp:
                hdrs = {k.lower(): v for k, v in resp.headers.items()}
                return resp.status, hdrs, resp.read().decode("utf-8", errors="replace")
        except urllib.error.HTTPError as exc:
            hdrs = {k.lower(): v for k, v in exc.headers.items()}
            return exc.code, hdrs, exc.read().decode("utf-8", errors="replace")
        except Exception as exc:
            last_err = exc
            time.sleep(0.25)
    raise RuntimeError(f"timed out requesting {method} {base}{path}: {last_err}")


def assert_gateway_headers(hdrs: dict[str, str], route: str) -> None:
    assert hdrs.get("x-content-type-options") == "nosniff", f"{route}: missing nosniff ({hdrs})"
    assert hdrs.get("x-frame-options") == "DENY", f"{route}: missing X-Frame-Options DENY ({hdrs})"
    assert hdrs.get("referrer-policy") == "no-referrer", f"{route}: missing Referrer-Policy ({hdrs})"


def extract_gauge(metrics_text: str, name: str) -> int:
    match = re.search(rf"^{re.escape(name)}\s+([0-9]+)\s*$", metrics_text, flags=re.MULTILINE)
    assert match is not None, f"metric `{name}` missing from exposition:\n{metrics_text}"
    return int(match.group(1))


# 1. Public site via Caddy gateway (/) + security headers + /dei crisis-first + /grafana/ 404.
pub_code, pub_hdrs, pub_body = request("/")
assert pub_code == 200, f"GET / expected 200, got {pub_code}"
assert_gateway_headers(pub_hdrs, "/")

dei_code, dei_hdrs, dei_body = request("/dei")
assert dei_code == 200, f"GET /dei expected 200, got {dei_code}"
assert_gateway_headers(dei_hdrs, "/dei")
idx_988, idx_911, idx_samhsa = dei_body.find("988"), dei_body.find("911"), dei_body.find("SAMHSA")
facility_markers = [
    dei_body.find(m)
    for m in ('id="facilities"', 'data-facility-id="', 'id="state-directory"', 'class="facility')
    if dei_body.find(m) >= 0
]
assert min(idx_988, idx_911, idx_samhsa) >= 0 and facility_markers
assert max(idx_988, idx_911, idx_samhsa) < min(facility_markers)

graf_code, _, _ = request("/grafana/")
assert graf_code == 404, f"GET /grafana/ expected 404, got {graf_code}"

# 2. Clinic Wasm shell + /api/health + /api/metrics via Caddy gateway.
clinic_shell_code, clinic_shell_hdrs, clinic_shell_body = request("/elysianfields/")
assert clinic_shell_code == 200, f"GET /elysianfields/ expected 200, got {clinic_shell_code}"
assert_gateway_headers(clinic_shell_hdrs, "/elysianfields/")
assert "/elysianfields" in clinic_shell_body

clinic_health_code, _, clinic_health_body = request("/elysianfields/api/health")
assert clinic_health_code == 200, f"GET /elysianfields/api/health expected 200, got {clinic_health_code}"
clinic_health_json = json.loads(clinic_health_body)
assert clinic_health_json.get("db_healthy") is True, f"clinic health not db_healthy: {clinic_health_json}"

# 3. Mobile PWA + /mobile-api/readyz + /mobile-api/metrics via Caddy gateway.
mobile_pwa_code, mobile_pwa_hdrs, _ = request("/elysianfields/mobile/")
assert mobile_pwa_code == 200, f"GET /elysianfields/mobile/ expected 200, got {mobile_pwa_code}"
assert_gateway_headers(mobile_pwa_hdrs, "/elysianfields/mobile/")

mobile_readyz_code, _, mobile_readyz_body = request("/elysianfields/mobile-api/readyz")
assert mobile_readyz_code == 200, f"GET /elysianfields/mobile-api/readyz expected 200, got {mobile_readyz_code}"
mobile_readyz_json = json.loads(mobile_readyz_body)
assert mobile_readyz_json.get("ready") is True
assert mobile_readyz_json.get("restore_replay_verified") is True

mobile_metrics_code, _, mobile_metrics_body = request("/elysianfields/mobile-api/metrics")
assert mobile_metrics_code == 200, f"GET /elysianfields/mobile-api/metrics expected 200, got {mobile_metrics_code}"
assert extract_gauge(mobile_metrics_body, "ef_mobile_db_ready") == 1

# 4. Live Clinic auth write -> PG18 durable WAL -> unauthenticated prompt rejected (401) -> container restart replay.
probe_email = "compose-e2e-probe@clinic.example"
auth_code, _, auth_body = request(
    "/elysianfields/api/auth/request-link",
    method="POST",
    body={"email": probe_email},
)
assert auth_code == 200, f"POST /elysianfields/api/auth/request-link expected 200, got {auth_code}: {auth_body}"

unauth_prompt_code, _, _ = request(
    "/elysianfields/api/patients/ws-1/prompts",
    method="POST",
    body={"prompt": "Summarize recent lab trend"},
)
assert unauth_prompt_code == 401, f"unauthenticated prompt expected 401, got {unauth_prompt_code}"

clinic_metrics_code, _, clinic_metrics_body = request("/elysianfields/api/metrics")
assert clinic_metrics_code == 200, f"GET /elysianfields/api/metrics expected 200, got {clinic_metrics_code}"
assert extract_gauge(clinic_metrics_body, "ef_clinic_db_ready") == 1
wal_seq_before_restart = extract_gauge(clinic_metrics_body, "ef_clinic_wal_seq")
assert wal_seq_before_restart >= 1, f"expected ef_clinic_wal_seq >= 1 after auth write, got {wal_seq_before_restart}"
assert probe_email not in clinic_metrics_body, "PHI/email leaked into clinic /metrics!"
assert probe_email not in mobile_metrics_body, "PHI/email leaked into mobile /metrics!"

# Verify durable row directly in PostgreSQL 18 `ef_clinic` database.
pg_user = os.environ.get("EF_PG_USER", "elysian")
pg_rows_raw = subprocess.check_output(
    [
        "docker",
        "compose",
        "-p",
        project,
        "-f",
        "deploy/docker-compose.yml",
        "--profile",
        "gateway",
        "exec",
        "-T",
        "postgres",
        "psql",
        "-U",
        pg_user,
        "-d",
        "ef_clinic",
        "-tAc",
        "SELECT count(*) FROM ef_clinic_wal_events;",
    ],
    text=True,
).strip()
pg_wal_rows = int(pg_rows_raw)
assert pg_wal_rows >= 1, f"expected >= 1 durable row in ef_clinic.ef_clinic_wal_events, got {pg_wal_rows}"

# Restart clinic container and confirm PG18 WAL replay restores ef_clinic_wal_seq.
subprocess.check_call(
    [
        "docker",
        "compose",
        "-p",
        project,
        "-f",
        "deploy/docker-compose.yml",
        "--profile",
        "gateway",
        "restart",
        "clinic",
    ]
)
restarted_health_code, _, _ = request("/elysianfields/api/health", timeout=30.0)
assert restarted_health_code == 200, f"clinic health after restart expected 200, got {restarted_health_code}"
restarted_metrics_code, _, restarted_metrics_body = request("/elysianfields/api/metrics")
assert restarted_metrics_code == 200
assert extract_gauge(restarted_metrics_body, "ef_clinic_db_ready") == 1
wal_seq_after_restart = extract_gauge(restarted_metrics_body, "ef_clinic_wal_seq")
assert wal_seq_after_restart >= wal_seq_before_restart, (
    f"expected WAL replay seq >= {wal_seq_before_restart} after restart, got {wal_seq_after_restart}"
)

report = {
    "status": "passed",
    "gateway_origin": base,
    "project": project,
    "revision": os.environ.get("GITHUB_SHA", ""),
    "checks": {
        "public_root_status": pub_code,
        "public_dei_crisis_first": True,
        "public_grafana_404": graf_code == 404,
        "gateway_security_headers": True,
        "clinic_shell_status": clinic_shell_code,
        "clinic_health_db_healthy": True,
        "clinic_metrics_db_ready": 1,
        "clinic_auth_request_link_status": auth_code,
        "clinic_unauth_prompt_status": unauth_prompt_code,
        "clinic_pg18_wal_rows": pg_wal_rows,
        "clinic_wal_seq_before_restart": wal_seq_before_restart,
        "clinic_wal_seq_after_restart": wal_seq_after_restart,
        "mobile_pwa_status": mobile_pwa_code,
        "mobile_readyz_verified": True,
        "mobile_metrics_db_ready": 1,
        "zero_phi_in_metrics": True,
    },
}
rendered = json.dumps(report, indent=2) + "\n"
if report_path is not None:
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(rendered, encoding="utf-8")
print(rendered)
PY

echo "==> 5-container Compose + Caddy gateway E2E PASSED."
