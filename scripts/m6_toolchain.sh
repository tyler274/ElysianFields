#!/usr/bin/env bash
# Config-only opt-in and switch-back helper for ElyMalloc and ElyLD.
#
# Usage:
#   scripts/m6_toolchain.sh --mode default|elymalloc|elyld|both [options] [-- <command...>]
#
# Options:
#   --mode <mode>            Toolchain mode: default, elymalloc, elyld, or both (default: default).
#   --gc-mode <gc>           ElyLD linker mode: gc-sections (default) or incremental-no-gc.
#   --elymalloc-lib <path>   Path to pinned libmimalloc.so (or ELYSIAN_ELYMALLOC_LIB).
#   --elyld-bin <path>       Path to pinned elyld binary (or ELYSIAN_ELYLD_BIN).
#   --isolate-preload        Mask host /etc/ld-nix.so.preload via bwrap so exactly one allocator runs.
#   --print-env              Print shell export/unset statements instead of running a command.
#   -h, --help               Show this help text.
#
# Pinned upstream revisions (no forks carried in this repository):
#   ElyMalloc: github:tyler274/ElyMalloc @ cce11466569646e4e1f7afe9b44c6c80dd613b9f
#   ElyLD:     github:tyler274/ElyLD     @ 1b1e263ff40d5eea10966f6d2651f50ce43831ab
set -euo pipefail

mode="default"
gc_mode="gc-sections"
elymalloc_lib="${ELYSIAN_ELYMALLOC_LIB:-}"
elyld_bin="${ELYSIAN_ELYLD_BIN:-}"
isolate_preload=0
print_env=0
cmd=()

usage() {
  sed -n '2,18p' "$0"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --mode)
      shift
      mode="${1:?--mode requires one of: default, elymalloc, elyld, both}"
      ;;
    --mode=*)
      mode="${1#--mode=}"
      ;;
    --gc-mode)
      shift
      gc_mode="${1:?--gc-mode requires one of: gc-sections, incremental-no-gc}"
      ;;
    --gc-mode=*)
      gc_mode="${1#--gc-mode=}"
      ;;
    --elymalloc-lib)
      shift
      elymalloc_lib="${1:?--elymalloc-lib requires a file path}"
      ;;
    --elymalloc-lib=*)
      elymalloc_lib="${1#--elymalloc-lib=}"
      ;;
    --elyld-bin)
      shift
      elyld_bin="${1:?--elyld-bin requires an executable path}"
      ;;
    --elyld-bin=*)
      elyld_bin="${1#--elyld-bin=}"
      ;;
    --isolate-preload)
      isolate_preload=1
      ;;
    --print-env)
      print_env=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      cmd=("$@")
      break
      ;;
    *)
      echo "m6_toolchain.sh: unknown argument '$1'" >&2
      exit 2
      ;;
  esac
  shift
done

case "$mode" in
  default|elymalloc|elyld|both) ;;
  *)
    echo "m6_toolchain.sh: invalid --mode '$mode' (expected default, elymalloc, elyld, or both)" >&2
    exit 2
    ;;
esac

case "$gc_mode" in
  gc-sections|incremental-no-gc) ;;
  *)
    echo "m6_toolchain.sh: invalid --gc-mode '$gc_mode' (expected gc-sections or incremental-no-gc)" >&2
    exit 2
    ;;
esac

if [ "$print_env" -eq 0 ] && [ "${#cmd[@]}" -eq 0 ]; then
  echo "m6_toolchain.sh: specify --print-env or a command after '--'" >&2
  exit 2
fi

use_elymalloc=0
use_elyld=0
case "$mode" in
  elymalloc) use_elymalloc=1 ;;
  elyld)     use_elyld=1 ;;
  both)      use_elymalloc=1; use_elyld=1 ;;
esac

if [ "$use_elymalloc" -eq 1 ]; then
  if [ -z "$elymalloc_lib" ]; then
    echo "m6_toolchain.sh: --elymalloc-lib or ELYSIAN_ELYMALLOC_LIB is required for mode '$mode'" >&2
    exit 2
  fi
  if [ ! -f "$elymalloc_lib" ]; then
    echo "m6_toolchain.sh: ElyMalloc shared library not found: $elymalloc_lib" >&2
    exit 2
  fi
fi

if [ "$use_elyld" -eq 1 ]; then
  if [ -z "$elyld_bin" ]; then
    echo "m6_toolchain.sh: --elyld-bin or ELYSIAN_ELYLD_BIN is required for mode '$mode'" >&2
    exit 2
  fi
  if [ ! -x "$elyld_bin" ]; then
    echo "m6_toolchain.sh: ElyLD executable not found or not executable: $elyld_bin" >&2
    exit 2
  fi
fi

# Build config-only RUSTFLAGS for ElyLD.
#
# Trade-off between gc-sections and incremental developer builds:
#   - gc-sections (default): preserves dead-code elimination; ElyLD falls back
#     to a full padded link rather than patching in-place.
#   - incremental-no-gc: passes -Wl,--no-gc-sections -Wl,--incremental so ElyLD
#     enables fast in-place ELF patching on iterative local dev relinks.
elyld_rustflags=""
if [ "$use_elyld" -eq 1 ]; then
  elyld_rustflags="-C linker=clang -C link-arg=--ld-path=${elyld_bin}"
  if [ "$gc_mode" = "incremental-no-gc" ]; then
    elyld_rustflags="${elyld_rustflags} -C link-arg=-Wl,--no-gc-sections -C link-arg=-Wl,--incremental"
  fi
fi

shell_quote() {
  printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

if [ "$print_env" -eq 1 ]; then
  printf 'export ELYSIAN_M6_MODE=%s\n' "$(shell_quote "$mode")"
  printf 'export ELYSIAN_M6_GC_MODE=%s\n' "$(shell_quote "$gc_mode")"
  if [ "$use_elymalloc" -eq 1 ]; then
    printf 'export LD_PRELOAD=%s\n' "$(shell_quote "$elymalloc_lib")"
  else
    printf 'unset LD_PRELOAD\n'
  fi
  if [ "$use_elyld" -eq 1 ]; then
    printf 'export RUSTFLAGS=%s\n' "$(shell_quote "$elyld_rustflags")"
  else
    printf 'unset RUSTFLAGS\n'
  fi
  exit 0
fi

export ELYSIAN_M6_MODE="$mode"
export ELYSIAN_M6_GC_MODE="$gc_mode"

if [ "$use_elyld" -eq 1 ]; then
  export RUSTFLAGS="$elyld_rustflags"
else
  unset RUSTFLAGS || true
fi

if [ "$use_elymalloc" -eq 1 ]; then
  export LD_PRELOAD="$elymalloc_lib"
else
  unset LD_PRELOAD || true
fi

if [ "$isolate_preload" -eq 1 ]; then
  preload_file=""
  if [ -e /etc/ld-nix.so.preload ]; then
    preload_file="$(readlink -f /etc/ld-nix.so.preload)"
  elif [ -e /etc/ld.so.preload ]; then
    preload_file="$(readlink -f /etc/ld.so.preload)"
  fi

  if [ -n "$preload_file" ]; then
    bwrap_prefix=()
    if command -v bwrap >/dev/null 2>&1; then
      bwrap_prefix=(bwrap)
    elif command -v nix >/dev/null 2>&1; then
      bwrap_prefix=(nix shell nixpkgs#bubblewrap -c bwrap)
    else
      echo "m6_toolchain.sh: --isolate-preload requested and $preload_file exists, but bwrap is unavailable" >&2
      exit 1
    fi

    bwrap_args=(--dev-bind / / --ro-bind /dev/null "$preload_file")
    if [ "$use_elymalloc" -eq 1 ]; then
      bwrap_args+=(--setenv LD_PRELOAD "$elymalloc_lib")
    else
      bwrap_args+=(--unsetenv LD_PRELOAD)
    fi
    exec "${bwrap_prefix[@]}" "${bwrap_args[@]}" "${cmd[@]}"
  fi
fi

exec "${cmd[@]}"
