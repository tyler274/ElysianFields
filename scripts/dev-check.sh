#!/usr/bin/env bash
# Single local verification command matching Elysian Fields CI.
#
# Usage:
#   scripts/dev-check.sh [--tier touched|pre-commit|full]
#
# Tiers:
#   touched     (default) Format check, Clippy & tests for changed crates, Python script tests.
#   pre-commit  Staged format check, Clippy for staged crates (skips cargo test), Python tests.
#   full        Full workspace fmt, host + wasm32 Clippy, cargo test --locked, Python tests,
#               and book / wall checks when present.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

tier="touched"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --tier)
      shift
      tier="${1:?--tier requires one of: touched, pre-commit, full}"
      ;;
    --tier=*)
      tier="${1#--tier=}"
      ;;
    -h|--help)
      sed -n '2,11p' "$0"
      exit 0
      ;;
    *)
      echo "dev-check.sh: unknown argument '$1'" >&2
      exit 2
      ;;
  esac
  shift
done

case "$tier" in
  touched|pre-commit|full) ;;
  *)
    echo "dev-check.sh: invalid tier '$tier' (expected touched, pre-commit, or full)" >&2
    exit 2
    ;;
esac

stage_names=()
stage_statuses=()
stage_durations=()
overall_rc=0
total_start_secs="$SECONDS"

format_duration() {
  local secs="$1"
  if [ "$secs" -ge 60 ]; then
    printf '%dm%02ds' $((secs / 60)) $((secs % 60))
  else
    printf '%ds' "$secs"
  fi
}

print_summary() {
  local exit_rc=$?
  if [ "${#stage_names[@]}" -gt 0 ]; then
    local total_elapsed=$((SECONDS - total_start_secs))
    printf '\n%-26s %-8s %s\n' 'Stage' 'Result' 'Duration'
    printf '%-26s %-8s %s\n' '--------------------------' '------' '--------'
    local i
    for ((i = 0; i < ${#stage_names[@]}; i++)); do
      printf '%-26s %-8s %s\n' \
        "${stage_names[$i]}" \
        "${stage_statuses[$i]}" \
        "$(format_duration "${stage_durations[$i]}")"
    done
    printf '%-26s %-8s %s\n' '--------------------------' '------' '--------'
    local overall_label="PASS"
    if [ "$overall_rc" -ne 0 ] || [ "$exit_rc" -ne 0 ]; then
      overall_label="FAIL"
    fi
    printf '%-26s %-8s %s\n' 'TOTAL' "$overall_label" "$(format_duration "$total_elapsed")"
  fi
}
trap print_summary EXIT

run_stage() {
  local name="$1"
  shift
  printf '\n==> [dev-check:%s] %s\n' "$tier" "$name"
  local start_secs="$SECONDS"
  local rc=0
  set +e
  "$@"
  rc=$?
  set -e
  local elapsed=$((SECONDS - start_secs))
  stage_names+=("$name")
  stage_durations+=("$elapsed")
  if [ "$rc" -eq 0 ]; then
    stage_statuses+=("PASS")
    return 0
  fi
  stage_statuses+=("FAIL($rc)")
  overall_rc="$rc"
  return "$rc"
}

run_python() {
  if command -v python3 >/dev/null 2>&1; then
    python3 "$@"
  elif command -v nix >/dev/null 2>&1; then
    nix shell nixpkgs#python3 -c python3 "$@"
  else
    echo "dev-check.sh: python3 not found on PATH and nix is unavailable" >&2
    return 1
  fi
}

has_wasm_target() {
  if command -v rustc >/dev/null 2>&1; then
    local libdir
    libdir=$(rustc --print target-libdir --target wasm32-unknown-unknown 2>/dev/null || true)
    if [ -n "$libdir" ] && [ -d "$libdir" ]; then
      return 0
    fi
  fi
  if command -v rustup >/dev/null 2>&1; then
    rustup target list --installed 2>/dev/null | grep -qx 'wasm32-unknown-unknown' && return 0
  fi
  return 1
}

run_wasm_clippy() {
  if has_wasm_target; then
    cargo clippy --workspace --target wasm32-unknown-unknown -- -D warnings
  else
    echo "dev-check.sh: skipping wasm32 Clippy (wasm32-unknown-unknown target not installed)" >&2
    if [ "${ELYSIAN_REQUIRE_WASM:-0}" = "1" ]; then
      return 1
    fi
  fi
}

run_python_tests() {
  run_python -m unittest discover -s scripts -p 'test_*.py' -v
}

run_optional_doc_and_wall_checks() {
  if [ -f scripts/check_wall.py ]; then
    run_python scripts/check_wall.py
  elif [ -f scripts/check-wall.sh ]; then
    bash scripts/check-wall.sh
  fi
  if [ -f book/book.toml ] && command -v mdbook >/dev/null 2>&1; then
    mdbook build book >/dev/null
  fi
  if command -v lychee >/dev/null 2>&1 && [ -d docs ]; then
    lychee --offline --no-progress --root-dir "$root" 'docs/**/*.md'
  fi
}

run_stage "cargo-fmt" cargo fmt --all --check
run_stage "python-tests" run_python_tests

if [ "$tier" = "full" ]; then
  run_stage "clippy-host" cargo clippy --workspace --all-targets -- -D warnings
  run_stage "clippy-wasm" run_wasm_clippy
  run_stage "cargo-test" cargo test --locked --workspace
  run_stage "docs-and-wall" run_optional_doc_and_wall_checks
  exit "$overall_rc"
fi

# Collect changed files for touched / pre-commit tiers.
if [ "$tier" = "pre-commit" ]; then
  changed_files=$(git diff --cached --name-only --diff-filter=ACMR)
else
  changed_files=$(git status --porcelain=v1 | awk '{print $NF}')
  if [ -z "$changed_files" ]; then
    changed_files=$(git diff --name-only --diff-filter=ACMR HEAD~1..HEAD 2>/dev/null || true)
  fi
fi

needs_workspace=0
pkg_list=""

crate_package_name() {
  local manifest="$1/Cargo.toml"
  [ -f "$manifest" ] || return 1
  awk -F'"' '/^name[[:space:]]*=/{print $2; exit}' "$manifest"
}

while IFS= read -r path; do
  [ -n "$path" ] || continue
  case "$path" in
    Cargo.toml|Cargo.lock|rust-toolchain.toml|deny.toml|flake.nix|flake.lock)
      needs_workspace=1
      ;;
    crates/*)
      seg="${path#crates/}"
      crate_dir="crates/${seg%%/*}"
      if pkg=$(crate_package_name "$crate_dir"); then
        case " $pkg_list " in
          *" $pkg "*) ;;
          *) pkg_list="$pkg_list $pkg" ;;
        esac
      else
        needs_workspace=1
      fi
      ;;
  esac
done <<<"$changed_files"

if [ "$needs_workspace" = 1 ]; then
  run_stage "clippy-host" cargo clippy --workspace --all-targets -- -D warnings
  run_stage "clippy-wasm" run_wasm_clippy
  if [ "$tier" != "pre-commit" ]; then
    run_stage "cargo-test" cargo test --locked --workspace
  fi
  run_stage "docs-and-wall" run_optional_doc_and_wall_checks
  exit "$overall_rc"
fi

pkg_count=$(wc -w <<<"$pkg_list")
if [ "$pkg_count" -eq 0 ]; then
  run_stage "docs-and-wall" run_optional_doc_and_wall_checks
  exit "$overall_rc"
fi

pkg_args=()
for pkg in $pkg_list; do
  pkg_args+=(-p "$pkg")
done

run_stage "clippy-host" cargo clippy "${pkg_args[@]}" --all-targets -- -D warnings
run_stage "clippy-wasm" run_wasm_clippy
if [ "$tier" != "pre-commit" ]; then
  run_stage "cargo-test" cargo test --locked "${pkg_args[@]}"
fi
run_stage "docs-and-wall" run_optional_doc_and_wall_checks
exit "$overall_rc"
