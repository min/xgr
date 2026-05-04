#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_DIR="${BENCH_DIR:-"$ROOT_DIR/.context/bench/public-xcodegen"}"
RUNS="${RUNS:-5}"
ONLY=""
DO_CLONE=1
DO_BENCH=1

XCODEGEN_BIN="${XCODEGEN_BIN:-xcodegen}"
XGR_BIN="${XGR_BIN:-"$ROOT_DIR/target/release/xgr"}"

# name|repo url|branch|spec path
CANDIDATES=(
  "provenance|https://github.com/Provenance-Emu/Provenance.git|develop|project.yml"
  "tutanota-mail|https://github.com/tutao/tutanota.git|master|app-ios/mail-project.yml"
  "tutanota-calendar|https://github.com/tutao/tutanota.git|master|app-ios/calendar-project.yml"
  "element-ios|https://github.com/element-hq/element-ios.git|develop|project.yml"
  "mapbox-maps-ios|https://github.com/mapbox/mapbox-maps-ios.git|main|project.yml"
  "kiwix-apple|https://github.com/kiwix/kiwix-apple.git|main|project.yml"
)

usage() {
  cat <<'EOF'
Usage: scripts/bench_public_xcodegen.sh [options]

Clone public XcodeGen repos under .context/bench, generate each project with
upstream XcodeGen and xgr, compare generated output byte-for-byte, and run
hyperfine timing benchmarks when available.

Options:
  --only NAME       Run one candidate from the manifest.
  --runs N         hyperfine runs per generator. Default: 5.
  --no-clone       Use existing checkouts in BENCH_DIR/repos.
  --no-bench       Do parity generation only.
  -h, --help       Show this help.

Environment:
  BENCH_DIR        Artifact root. Default: .context/bench/public-xcodegen
  XCODEGEN_BIN     Upstream XcodeGen executable. Default: xcodegen
  XGR_BIN          xgr executable. Default: target/release/xgr
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --only)
      ONLY="${2:?missing candidate name}"
      shift 2
      ;;
    --runs)
      RUNS="${2:?missing run count}"
      shift 2
      ;;
    --no-clone)
      DO_CLONE=0
      shift
      ;;
    --no-bench)
      DO_BENCH=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

quote() {
  printf "%q" "$1"
}

repo_slug() {
  local url="$1"
  basename "$url" .git
}

prepare_copy() {
  local source_dir="$1"
  local dest_dir="$2"
  rm -rf "$dest_dir"
  mkdir -p "$(dirname "$dest_dir")"
  rsync -a --delete --exclude .git "$source_dir/" "$dest_dir/"
  find "$dest_dir" -name "*.xcodeproj" -type d -prune -exec rm -rf {} +
}

generated_project_dir() {
  local work_dir="$1"
  find "$work_dir" -name "*.xcodeproj" -type d -prune | sort | head -n 1
}

project_name_from_xgr() {
  local spec="$1"
  "$XGR_BIN" dump --spec "$spec" \
    | /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin)["name"])'
}

run_upstream_once() {
  local source_dir="$1"
  local spec_rel="$2"
  local work_dir="$3"
  prepare_copy "$source_dir" "$work_dir"
  (cd "$work_dir" && "$XCODEGEN_BIN" generate --spec "$spec_rel" --quiet)
  local project_dir
  project_dir="$(generated_project_dir "$work_dir")"
  if [[ -z "$project_dir" ]]; then
    echo "upstream XcodeGen did not produce an .xcodeproj for $spec_rel" >&2
    return 1
  fi
  printf "%s\n" "$project_dir"
}

run_xgr_once() {
  local source_dir="$1"
  local spec_rel="$2"
  local work_dir="$3"
  prepare_copy "$source_dir" "$work_dir"
  local spec_path="$work_dir/$spec_rel"
  local project_name
  project_name="$(project_name_from_xgr "$spec_path")"
  "$XGR_BIN" generate --spec "$spec_path" --output "$work_dir/$project_name.xcodeproj" >/dev/null
  printf "%s\n" "$work_dir/$project_name.xcodeproj"
}

benchmark_case() {
  local name="$1"
  local source_dir="$2"
  local spec_rel="$3"
  local project_name="$4"
  local bench_tmp="$BENCH_DIR/hyperfine/$name"
  local upstream_work="$bench_tmp/upstream"
  local xgr_work="$bench_tmp/xgr"
  local spec_q xcodegen_q xgr_bin_q upstream_work_q xgr_work_q output_q

  prepare_copy "$source_dir" "$upstream_work"
  prepare_copy "$source_dir" "$xgr_work"

  upstream_work_q="$(quote "$upstream_work")"
  xgr_work_q="$(quote "$xgr_work")"
  spec_q="$(quote "$spec_rel")"
  xcodegen_q="$(quote "$XCODEGEN_BIN")"
  xgr_bin_q="$(quote "$XGR_BIN")"
  output_q="$(quote "$xgr_work/$project_name.xcodeproj")"

  mkdir -p "$BENCH_DIR/results"
  hyperfine --runs "$RUNS" --warmup 1 \
    --export-json "$BENCH_DIR/results/$name.upstream.hyperfine.json" \
    "find $upstream_work_q -name '*.xcodeproj' -type d -prune -exec rm -rf {} + && cd $upstream_work_q && $xcodegen_q generate --spec $spec_q --quiet"
  hyperfine --runs "$RUNS" --warmup 1 \
    --export-json "$BENCH_DIR/results/$name.xgr.hyperfine.json" \
    "find $xgr_work_q -name '*.xcodeproj' -type d -prune -exec rm -rf {} + && $xgr_bin_q generate --spec $xgr_work_q/$spec_q --output $output_q >/dev/null"
}

require_cmd git
require_cmd rsync
require_cmd "$XCODEGEN_BIN"

if [[ ! -x "$XGR_BIN" ]]; then
  (cd "$ROOT_DIR" && cargo build --release --locked)
fi

mkdir -p "$BENCH_DIR/repos" "$BENCH_DIR/runs" "$BENCH_DIR/diffs"

for entry in "${CANDIDATES[@]}"; do
  IFS="|" read -r name url branch spec_rel <<<"$entry"
  if [[ -n "$ONLY" && "$ONLY" != "$name" ]]; then
    continue
  fi

  repo_dir="$BENCH_DIR/repos/$(repo_slug "$url")"
  echo "==> $name"

  if [[ "$DO_CLONE" -eq 1 ]]; then
    if [[ ! -d "$repo_dir/.git" ]]; then
      git clone --filter=blob:none --depth 1 --branch "$branch" "$url" "$repo_dir"
    else
      git -C "$repo_dir" fetch --depth 1 origin "$branch"
      git -C "$repo_dir" checkout --detach FETCH_HEAD
    fi
  fi

  if [[ ! -f "$repo_dir/$spec_rel" ]]; then
    echo "missing spec: $repo_dir/$spec_rel" >&2
    continue
  fi

  if ! upstream_project="$(run_upstream_once "$repo_dir" "$spec_rel" "$BENCH_DIR/runs/$name/upstream")"; then
    echo "upstream XcodeGen failed for $name" >&2
    continue
  fi
  if ! xgr_project="$(run_xgr_once "$repo_dir" "$spec_rel" "$BENCH_DIR/runs/$name/xgr")"; then
    echo "xgr failed for $name" >&2
    continue
  fi
  project_name="$(basename "$xgr_project" .xcodeproj)"

  if cmp -s "$upstream_project/project.pbxproj" "$xgr_project/project.pbxproj"; then
    echo "pbxproj: byte-for-byte match"
  else
    echo "pbxproj: differs"
    diff -u "$upstream_project/project.pbxproj" "$xgr_project/project.pbxproj" \
      > "$BENCH_DIR/diffs/$name.project.pbxproj.diff" || true
    echo "diff: $BENCH_DIR/diffs/$name.project.pbxproj.diff"
  fi

  if diff -qr "$upstream_project" "$xgr_project" > "$BENCH_DIR/diffs/$name.xcodeproj.diff"; then
    echo "xcodeproj: byte-for-byte match"
    rm -f "$BENCH_DIR/diffs/$name.xcodeproj.diff"
  else
    echo "xcodeproj: differs"
    echo "diff: $BENCH_DIR/diffs/$name.xcodeproj.diff"
  fi

  if [[ "$DO_BENCH" -eq 1 ]]; then
    if command -v hyperfine >/dev/null 2>&1; then
      if ! benchmark_case "$name" "$repo_dir" "$spec_rel" "$project_name"; then
        echo "benchmark failed for $name" >&2
      fi
    else
      echo "hyperfine not found; skipping timing benchmark"
    fi
  fi
done
