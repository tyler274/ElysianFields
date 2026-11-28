#!/usr/bin/env bash
# Live 5-container integrated Docker Compose + Caddy single-origin gateway E2E
# verification (`postgres:18`, `clinic`, `mobile`, `public-site`, `gateway`).
#
# Usage:
#   scripts/verify_compose_stack.sh [--no-build] [--project <name>]
#     [--clinic-image <tag>] [--mobile-image <tag>] [--public-image <tag>]
#     [--subnet <cidr>] [--report <path.json>]
#
# Options:
#   --no-build             Reuse already-built local images instead of rebuilding.
#   --project <name>       Docker Compose project name (default: elysian-compose-e2e).
#   --clinic-image <tag>   Tag existing clinic image for Compose reuse under --project.
#   --mobile-image <tag>   Tag existing mobile image for Compose reuse under --project.
#   --public-image <tag>   Tag existing public-site image for Compose reuse under --project.
#   --subnet <cidr>        Explicit bridge subnet CIDR (or EF_COMPOSE_SUBNET) when default pools are full.
#   --report <path.json>   Optional path to write structured JSON verification output.
#   -h, --help             Show this usage text.
set -Eeuo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

no_build=0
project="elysian-compose-e2e"
clinic_image=""
mobile_image=""
public_image=""
subnet="${EF_COMPOSE_SUBNET:-}"
report_path=""
current_step="initialization"
last_failed_cmd=""
verify_log=""
override_file=""

usage() {
  sed -n '2,19p' "$0"
}

emit_error() {
  local msg="$1"
  if [ "${GITHUB_ACTIONS:-false}" = "true" ]; then
    echo "::error title=Compose Gateway E2E::${msg}"
  else
    echo "verify_compose_stack.sh ERROR: ${msg}" >&2
  fi
}

on_err() {
  case "${BASH_COMMAND:-}" in
    tee\ *|"") ;;
    *) last_failed_cmd="${BASH_COMMAND}" ;;
  esac
}
trap on_err ERR

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
    --subnet)
      shift
      subnet="${1:?--subnet requires a CIDR}"
      ;;
    --subnet=*)
      subnet="${1#--subnet=}"
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

compose_files=(-f deploy/docker-compose.yml)
override_file=$(mktemp "${TMPDIR:-/tmp}/verify-compose-override.XXXXXX.yml")
cat >"$override_file" <<'EOF'
services:
  clinic:
    environment:
      EF_CLINIC_SEED_DEMO: "1"
EOF
if [ -n "$subnet" ]; then
  cat >>"$override_file" <<EOF
networks:
  default:
    ipam:
      config:
        - subnet: ${subnet}
EOF
fi
compose_files+=(-f "$override_file")

compose_cmd=(docker compose -p "$project" "${compose_files[@]}" --profile gateway)
verify_log=$(mktemp "${TMPDIR:-/tmp}/verify-compose-stack.XXXXXX.log")

cleanup() {
  local rc=$?
  set +e
  if [ "$rc" -ne 0 ]; then
    local ps_summary last_err_line summary_cmd
    ps_summary=$("${compose_cmd[@]}" ps --format '{{.Service}}={{.State}}({{.Health}})' 2>/dev/null \
      | tr '\n' ' ' | sed -e 's/[[:space:]]\+/ /g' -e 's/ $//' || true)
    if [ -z "$ps_summary" ]; then
      ps_summary="none"
    fi
    last_err_line=""
    if [ -n "$verify_log" ] && [ -s "$verify_log" ]; then
      last_err_line=$(grep -v '^[[:space:]]*$' "$verify_log" | tail -n 1 | tr '\r\n' ' ' | cut -c1-240 || true)
    fi
    summary_cmd=$(printf '%s' "${last_failed_cmd:-unknown}" | tr '\r\n' ' ' | cut -c1-120)
    emit_error "step=${current_step}; exit=${rc}; cmd=${summary_cmd}; services=[${ps_summary}]; last_error=${last_err_line:-see logs}"
    "${compose_cmd[@]}" ps 2>&1 || true
    "${compose_cmd[@]}" logs --tail=60 2>&1 || true
  fi
  "${compose_cmd[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  if [ -n "$verify_log" ]; then
    rm -f "$verify_log"
  fi
  if [ -n "$override_file" ]; then
    rm -f "$override_file"
  fi
  exit "$rc"
}
trap cleanup EXIT INT TERM

current_step="image-tagging"
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
current_step="compose-reset"
"${compose_cmd[@]}" down -v --remove-orphans >/dev/null 2>&1 || true

up_args=(up -d --wait --wait-timeout 90)
if [ "$no_build" -eq 1 ]; then
  up_args+=(--no-build)
else
  up_args+=(--build)
fi

current_step="compose-up"
last_failed_cmd="docker compose up -d --wait"
echo "==> Starting 5-container Elysian Fields Compose + Caddy gateway stack (project=${project})"
"${compose_cmd[@]}" "${up_args[@]}" 2>&1 | tee "$verify_log"

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

current_step="gateway-e2e-assertions"
last_failed_cmd="python3 compose-gateway-e2e"
echo "==> Verifying single-origin Caddy gateway (http://127.0.0.1:8080) routes, headers, metrics, and PG18 persistence"
run_python - "$project" "${report_path}" <<'PY' 2>&1 | tee "$verify_log"
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
    headers: dict[str, str] | None = None,
    timeout: float = 20.0,
    retry_statuses: tuple[int, ...] = (),
) -> tuple[int, dict[str, str], str]:
    deadline = time.monotonic() + timeout
    last_err: Exception | None = None
    last_status: int | None = None
    data = json.dumps(body).encode("utf-8") if body is not None else None
    headers_in = {"Content-Type": "application/json"} if data is not None else {}
    if headers:
        headers_in.update(headers)
    while time.monotonic() < deadline:
        req = urllib.request.Request(f"{base}{path}", data=data, headers=headers_in, method=method)
        try:
            with urllib.request.urlopen(req, timeout=5.0) as resp:
                hdrs = {k.lower(): v for k, v in resp.headers.items()}
                return resp.status, hdrs, resp.read().decode("utf-8", errors="replace")
        except urllib.error.HTTPError as exc:
            hdrs = {k.lower(): v for k, v in exc.headers.items()}
            payload = exc.read().decode("utf-8", errors="replace")
            if exc.code in retry_statuses and time.monotonic() < deadline:
                last_status = exc.code
                time.sleep(0.25)
                continue
            return exc.code, hdrs, payload
        except Exception as exc:
            last_err = exc
            time.sleep(0.25)
    raise RuntimeError(f"timed out requesting {method} {base}{path}: last_status={last_status} last_err={last_err}")


REQUEST_ID_RE = re.compile(r"^[A-Za-z0-9._-]{1,64}$")


def assert_gateway_headers(hdrs: dict[str, str], route: str) -> None:
    assert hdrs.get("x-content-type-options") == "nosniff", f"{route}: missing nosniff ({hdrs})"
    assert hdrs.get("x-frame-options") == "DENY", f"{route}: missing X-Frame-Options DENY ({hdrs})"
    assert hdrs.get("referrer-policy") == "no-referrer", f"{route}: missing Referrer-Policy ({hdrs})"


def assert_request_id(hdrs: dict[str, str], route: str, expected: str | None = None) -> str:
    req_id = hdrs.get("x-request-id", "")
    assert REQUEST_ID_RE.match(req_id), f"{route}: missing or invalid X-Request-Id ({req_id!r}, hdrs={hdrs})"
    if expected is not None:
        assert req_id == expected, f"{route}: expected echoed X-Request-Id {expected!r}, got {req_id!r}"
    return req_id


def extract_gauge(metrics_text: str, name: str) -> int:
    match = re.search(rf"^{re.escape(name)}\s+([0-9]+)\s*$", metrics_text, flags=re.MULTILINE)
    assert match is not None, f"metric `{name}` missing from exposition:\n{metrics_text}"
    return int(match.group(1))


# 1. Public site via Caddy gateway (/) + security headers + X-Request-Id + /dei crisis-first + /grafana/ 404.
pub_code, pub_hdrs, pub_body = request("/")
assert pub_code == 200, f"GET / expected 200, got {pub_code}"
assert_gateway_headers(pub_hdrs, "/")
assert_request_id(pub_hdrs, "/")

dei_code, dei_hdrs, dei_body = request("/dei")
assert dei_code == 200, f"GET /dei expected 200, got {dei_code}"
assert_gateway_headers(dei_hdrs, "/dei")
assert_request_id(dei_hdrs, "/dei")
idx_988, idx_911, idx_samhsa = dei_body.find("988"), dei_body.find("911"), dei_body.find("SAMHSA")
facility_markers = [
    dei_body.find(m)
    for m in ('id="facilities"', 'data-facility-id="', 'id="state-directory"', 'class="facility')
    if dei_body.find(m) >= 0
]
assert min(idx_988, idx_911, idx_samhsa) >= 0 and facility_markers
assert max(idx_988, idx_911, idx_samhsa) < min(facility_markers)

dei_ca_code, dei_ca_hdrs, dei_ca_body = request("/dei?state=CA&service=crisis")
assert dei_ca_code == 200, f"GET /dei?state=CA&service=crisis expected 200, got {dei_ca_code}"
assert_gateway_headers(dei_ca_hdrs, "/dei?state=CA&service=crisis")
assert_request_id(dei_ca_hdrs, "/dei?state=CA&service=crisis")
assert "988" in dei_ca_body, "filtered /dei?state=CA&service=crisis missing pinned 988 banner"
assert (
    "Bay Area Crisis Stabilization" in dei_ca_body
    or "Harborview Behavioral Crisis Center" in dei_ca_body
), "filtered /dei?state=CA&service=crisis missing California crisis facility"
ca_fac_idx = dei_ca_body.find('id="facilities"')
assert ca_fac_idx >= 0 and dei_ca_body.find("988") < ca_fac_idx

dei_xss_code, dei_xss_hdrs, dei_xss_body = request("/dei?q=%3Cscript%3Ealert(1)%3C/script%3E")
assert dei_xss_code == 200, f"GET /dei?q=<script> expected 200, got {dei_xss_code}"
assert_gateway_headers(dei_xss_hdrs, "/dei?q=<script>")
assert_request_id(dei_xss_hdrs, "/dei?q=<script>")
assert "&lt;script&gt;" in dei_xss_body, "GET /dei?q=<script> did not HTML-escape <script>"
assert "<script>alert(1)" not in dei_xss_body, "raw <script>alert(1) leaked into /dei HTML"

graf_code, graf_hdrs, _ = request("/grafana/")
assert graf_code == 404, f"GET /grafana/ expected 404, got {graf_code}"
assert_request_id(graf_hdrs, "/grafana/")

# 2. Clinic Wasm shell + /api/health + /api/healthz + /api/metrics via Caddy gateway.
clinic_shell_code, clinic_shell_hdrs, clinic_shell_body = request("/elysianfields/")
assert clinic_shell_code == 200, f"GET /elysianfields/ expected 200, got {clinic_shell_code}"
assert_gateway_headers(clinic_shell_hdrs, "/elysianfields/")
assert_request_id(clinic_shell_hdrs, "/elysianfields/")
assert "/elysianfields" in clinic_shell_body

clinic_health_code, clinic_health_hdrs, clinic_health_body = request("/elysianfields/api/health")
assert clinic_health_code == 200, f"GET /elysianfields/api/health expected 200, got {clinic_health_code}"
assert_request_id(clinic_health_hdrs, "/elysianfields/api/health")
clinic_health_json = json.loads(clinic_health_body)
assert clinic_health_json.get("db_healthy") is True, f"clinic health not db_healthy: {clinic_health_json}"

clinic_healthz_code, clinic_healthz_hdrs, clinic_healthz_body = request("/elysianfields/api/healthz")
assert clinic_healthz_code == 200, f"GET /elysianfields/api/healthz expected 200, got {clinic_healthz_code}"
assert_request_id(clinic_healthz_hdrs, "/elysianfields/api/healthz")
assert json.loads(clinic_healthz_body).get("db_healthy") is True

# 3. Mobile PWA + /mobile-api/readyz + /mobile-api/healthz + /mobile-api/metrics via Caddy gateway.
mobile_pwa_code, mobile_pwa_hdrs, mobile_pwa_body = request("/elysianfields/mobile/")
assert mobile_pwa_code == 200, f"GET /elysianfields/mobile/ expected 200, got {mobile_pwa_code}"
assert_gateway_headers(mobile_pwa_hdrs, "/elysianfields/mobile/")
assert_request_id(mobile_pwa_hdrs, "/elysianfields/mobile/")
assert "/elysianfields/mobile" in mobile_pwa_body

mobile_manifest_code, mobile_manifest_hdrs, _ = request("/elysianfields/mobile/manifest.webmanifest")
assert mobile_manifest_code == 200, f"GET /elysianfields/mobile/manifest.webmanifest expected 200, got {mobile_manifest_code}"
assert "application/manifest+json" in mobile_manifest_hdrs.get("content-type", ""), (
    f"unexpected manifest content-type: {mobile_manifest_hdrs}"
)

mobile_sw_code, _, _ = request("/elysianfields/mobile/sw.js")
assert mobile_sw_code == 200, f"GET /elysianfields/mobile/sw.js expected 200, got {mobile_sw_code}"

mobile_readyz_code, mobile_readyz_hdrs, mobile_readyz_body = request("/elysianfields/mobile-api/readyz")
assert mobile_readyz_code == 200, f"GET /elysianfields/mobile-api/readyz expected 200, got {mobile_readyz_code}"
assert_request_id(mobile_readyz_hdrs, "/elysianfields/mobile-api/readyz")
mobile_readyz_json = json.loads(mobile_readyz_body)
assert mobile_readyz_json.get("ready") is True
assert mobile_readyz_json.get("restore_replay_verified") is True

mobile_healthz_code, mobile_healthz_hdrs, mobile_healthz_body = request("/elysianfields/mobile-api/healthz")
assert mobile_healthz_code == 200, f"GET /elysianfields/mobile-api/healthz expected 200, got {mobile_healthz_code}"
assert_request_id(mobile_healthz_hdrs, "/elysianfields/mobile-api/healthz")
assert json.loads(mobile_healthz_body).get("ready") is True

mobile_metrics_code, mobile_metrics_hdrs, mobile_metrics_body = request("/elysianfields/mobile-api/metrics")
assert mobile_metrics_code == 200, f"GET /elysianfields/mobile-api/metrics expected 200, got {mobile_metrics_code}"
assert_request_id(mobile_metrics_hdrs, "/elysianfields/mobile-api/metrics")
assert extract_gauge(mobile_metrics_body, "ef_mobile_db_ready") == 1

# 3b. End-to-end X-Request-Id trace propagation echo across all 3 backends through Caddy.
trace_id = "ef-compose-trace-42"
for trace_route in ("/", "/elysianfields/api/healthz", "/elysianfields/mobile-api/healthz"):
    tr_code, tr_hdrs, _ = request(trace_route, headers={"X-Request-Id": trace_id})
    assert tr_code == 200, f"trace probe GET {trace_route} expected 200, got {tr_code}"
    assert_request_id(tr_hdrs, trace_route, expected=trace_id)

# 4. Live Clinic auth write -> PG18 durable WAL -> unauthenticated prompt rejected (401) -> container restart replay.
probe_email = "compose-e2e-probe@clinic.example"
auth_code, _, auth_body = request(
    "/elysianfields/api/auth/request-link",
    method="POST",
    body={"email": probe_email},
)
assert auth_code == 200, f"POST /elysianfields/api/auth/request-link expected 200, got {auth_code}: {auth_body}"

unauth_prompt_code, _, _ = request(
    "/elysianfields/api/workspaces/ws-1/chats/chat-1/commands",
    method="POST",
    body={"prompt": "Summarize recent lab trend"},
)
assert unauth_prompt_code == 401, f"unauthenticated chat command expected 401, got {unauth_prompt_code}"

# 4b. Clinic Today workflow endpoint: 401 unauthenticated, 200 with items + scheduled_visits when authenticated.
unauth_today_code, unauth_today_hdrs, _ = request("/elysianfields/api/today")
assert unauth_today_code == 401, f"unauthenticated GET /elysianfields/api/today expected 401, got {unauth_today_code}"
assert_request_id(unauth_today_hdrs, "/elysianfields/api/today (unauth)")

demo_email = "demo@clinic.example"
demo_link_code, _, demo_link_body = request(
    "/elysianfields/api/auth/request-link",
    method="POST",
    body={"email": demo_email},
)
assert demo_link_code == 200, f"demo request-link expected 200, got {demo_link_code}: {demo_link_body}"
demo_token = json.loads(demo_link_body).get("token") or "tok-1"
demo_verify_code, _, demo_verify_body = request(
    "/elysianfields/api/auth/verify",
    method="POST",
    body={"email": demo_email, "token": demo_token},
)
assert demo_verify_code == 200, f"demo verify expected 200, got {demo_verify_code}: {demo_verify_body}"
clinic_session = json.loads(demo_verify_body)["session"]

auth_today_code, auth_today_hdrs, auth_today_body = request(
    "/elysianfields/api/today",
    headers={"Authorization": f"Bearer {clinic_session}"},
)
assert auth_today_code == 200, f"authenticated GET /elysianfields/api/today expected 200, got {auth_today_code}: {auth_today_body}"
assert_request_id(auth_today_hdrs, "/elysianfields/api/today")
today_json = json.loads(auth_today_body)
assert isinstance(today_json.get("items"), list) and len(today_json["items"]) >= 1, (
    f"authenticated /elysianfields/api/today missing items: {today_json}"
)
assert isinstance(today_json.get("scheduled_visits"), list) and len(today_json["scheduled_visits"]) >= 1, (
    f"authenticated /elysianfields/api/today missing scheduled_visits: {today_json}"
)

clinic_metrics_code, clinic_metrics_hdrs, clinic_metrics_body = request("/elysianfields/api/metrics")
assert clinic_metrics_code == 200, f"GET /elysianfields/api/metrics expected 200, got {clinic_metrics_code}"
assert_request_id(clinic_metrics_hdrs, "/elysianfields/api/metrics")
assert extract_gauge(clinic_metrics_body, "ef_clinic_db_ready") == 1
wal_seq_before_restart = extract_gauge(clinic_metrics_body, "ef_clinic_wal_seq")
assert wal_seq_before_restart >= 1, f"expected ef_clinic_wal_seq >= 1 after auth write, got {wal_seq_before_restart}"
assert probe_email not in clinic_metrics_body, "PHI/email leaked into clinic /metrics!"
assert demo_email not in clinic_metrics_body, "demo email leaked into clinic /metrics!"
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
restarted_health_code, _, _ = request(
    "/elysianfields/api/health",
    timeout=30.0,
    retry_statuses=(502, 503, 504),
)
assert restarted_health_code == 200, f"clinic health after restart expected 200, got {restarted_health_code}"
restarted_metrics_code, _, restarted_metrics_body = request(
    "/elysianfields/api/metrics",
    retry_statuses=(502, 503, 504),
)
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
        "public_dei_ca_crisis_status": dei_ca_code,
        "public_dei_xss_escaped": True,
        "public_grafana_404": graf_code == 404,
        "gateway_security_headers": True,
        "clinic_shell_status": clinic_shell_code,
        "clinic_health_db_healthy": True,
        "clinic_metrics_db_ready": 1,
        "clinic_auth_request_link_status": auth_code,
        "clinic_unauth_prompt_status": unauth_prompt_code,
        "clinic_today_unauth_status": unauth_today_code,
        "clinic_today_auth_status": auth_today_code,
        "clinic_today_items_count": len(today_json["items"]),
        "clinic_today_scheduled_visits_count": len(today_json["scheduled_visits"]),
        "clinic_pg18_wal_rows": pg_wal_rows,
        "clinic_wal_seq_before_restart": wal_seq_before_restart,
        "clinic_wal_seq_after_restart": wal_seq_after_restart,
        "mobile_pwa_status": mobile_pwa_code,
        "mobile_manifest_status": mobile_manifest_code,
        "mobile_sw_status": mobile_sw_code,
        "mobile_readyz_verified": True,
        "mobile_metrics_db_ready": 1,
        "request_id_propagation_verified": True,
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
