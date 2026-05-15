#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

DEFAULT_TARGETS=(
  "x86_64-unknown-linux-gnu"
  "x86_64-apple-darwin"
  "aarch64-apple-darwin"
  "x86_64-pc-windows-msvc"
)

TARGETS_ENV="${TARGETS:-}"
INSTALL_TARGETS=0
TARGETS=()

usage() {
  cat <<'USAGE'
Usage: scripts/local-cross-ci.sh [OPTIONS] [TARGET...]

Runs local Rust CI checks across Linux, macOS, and Windows targets:
  cargo fmt --all -- --check
  cargo check --target <target> --all-targets
  cargo clippy --target <target> --all-targets -- -D warnings
  cargo build --target <target>

Options:
  --install-targets  Install missing rustup targets before running checks.
  -h, --help         Show this help text.

Targets:
  If no TARGET arguments are supplied, these targets are used:
    x86_64-unknown-linux-gnu
    x86_64-apple-darwin
    aarch64-apple-darwin
    x86_64-pc-windows-msvc

Environment:
  TARGETS="..."      Whitespace-separated target list. Ignored when TARGET
                     arguments are supplied.
  CARGO_FLAGS="..."  Extra flags passed to cargo check/clippy/build, for
                     example: CARGO_FLAGS="--locked".
  CLIPPY_FLAGS="..." Flags passed after cargo clippy's -- separator. Defaults
                     to "-D warnings". Set CLIPPY_FLAGS="" to only warn.
USAGE
}

while (($#)); do
  case "$1" in
    --install-targets)
      INSTALL_TARGETS=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      while (($#)); do
        TARGETS+=("$1")
        shift
      done
      ;;
    -*)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      TARGETS+=("$1")
      shift
      ;;
  esac
done

if ((${#TARGETS[@]} == 0)); then
  if [[ -n "${TARGETS_ENV}" ]]; then
    read -r -a TARGETS <<<"${TARGETS_ENV}"
  else
    TARGETS=("${DEFAULT_TARGETS[@]}")
  fi
fi

if ((${#TARGETS[@]} == 0)); then
  echo "error: no targets configured" >&2
  exit 2
fi

require_tool() {
  local tool="$1"
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "error: required tool not found in PATH: ${tool}" >&2
    exit 127
  fi
}

run() {
  printf '\n+'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

ensure_targets() {
  if ! command -v rustup >/dev/null 2>&1; then
    echo "warning: rustup not found; skipping target installation checks" >&2
    return
  fi

  local installed
  installed="$(rustup target list --installed)"

  local missing=()
  local target
  for target in "${TARGETS[@]}"; do
    if ! grep -Fxq "${target}" <<<"${installed}"; then
      missing+=("${target}")
    fi
  done

  if ((${#missing[@]} == 0)); then
    return
  fi

  if ((INSTALL_TARGETS)); then
    run rustup target add "${missing[@]}"
    return
  fi

  echo "error: missing Rust target(s): ${missing[*]}" >&2
  echo "hint: rerun with --install-targets, or run: rustup target add ${missing[*]}" >&2
  exit 1
}

require_tool cargo
require_tool rustc

cd "${REPO_ROOT}"

ensure_targets

read -r -a EXTRA_CARGO_FLAGS <<<"${CARGO_FLAGS:-}"
if [[ -v CLIPPY_FLAGS ]]; then
  CLIPPY_FLAGS_ENV="${CLIPPY_FLAGS}"
else
  CLIPPY_FLAGS_ENV="-D warnings"
fi
read -r -a EXTRA_CLIPPY_FLAGS <<<"${CLIPPY_FLAGS_ENV}"

echo "targets: ${TARGETS[*]}"

run cargo fmt --all -- --check

for target in "${TARGETS[@]}"; do
  echo
  echo "==> ${target}: cargo check"
  run cargo check "${EXTRA_CARGO_FLAGS[@]}" --target "${target}" --all-targets

  echo
  echo "==> ${target}: cargo clippy"
  clippy_cmd=(cargo clippy "${EXTRA_CARGO_FLAGS[@]}" --target "${target}" --all-targets)
  if ((${#EXTRA_CLIPPY_FLAGS[@]} > 0)); then
    clippy_cmd+=(-- "${EXTRA_CLIPPY_FLAGS[@]}")
  fi
  run "${clippy_cmd[@]}"

  echo
  echo "==> ${target}: cargo build"
  run cargo build "${EXTRA_CARGO_FLAGS[@]}" --target "${target}"
done
