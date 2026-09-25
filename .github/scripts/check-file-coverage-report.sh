#!/usr/bin/env bash
set -euo pipefail

minimum="${1:-90}"
report="${2:-coverage.json}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
workspace="${3:-$script_dir/../..}"
workspace_root="$(cd "$workspace" && pwd -P)/"
# Gate handwritten production files under crates/<package>/src/. Integration
# tests, colocated test.rs modules, generated macro code in vendor/, and other
# worktrees are not production files. The browser tests still run and exercise
# the production files counted here.
source_root="${workspace_root}crates/"

covered_files="$(jq --arg source_root "$source_root" '
  [
    .data[].files[]
    | select(.filename | startswith($source_root))
    | select(.filename | contains("/src/"))
    | select(.filename | endswith("/test.rs") | not)
    | select(.filename | endswith("/tests.rs") | not)
    | select(.summary.lines.count > 0)
  ]
  | length
' "$report")"

if [[ "$covered_files" -eq 0 ]]; then
  echo "coverage report contains no production files with executable lines under crates/" >&2
  exit 1
fi

summary="$(jq -r --arg workspace_root "$workspace_root" --arg source_root "$source_root" '
  .data[].files[]
  | select(.filename | startswith($source_root))
  | select(.filename | contains("/src/"))
  | select(.filename | endswith("/test.rs") | not)
  | select(.filename | endswith("/tests.rs") | not)
  | select(.summary.lines.count > 0)
  | [
      (.filename | ltrimstr($workspace_root)),
      (.summary.lines.percent | tostring),
      (.summary.lines.covered | tostring),
      (.summary.lines.count | tostring)
    ]
  | @tsv
' "$report")"

printf 'File\tLine coverage\tCovered lines\tCoverable lines\n'
while IFS=$'\t' read -r file percent covered count; do
  printf '%s\t%.2f%%\t%s\t%s\n' "$file" "$percent" "$covered" "$count"
done <<< "$summary"

if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  {
    printf '### Per-file line coverage\n\n'
    printf '| File | Coverage | Lines |\n'
    printf '| --- | ---: | ---: |\n'
    while IFS=$'\t' read -r file percent covered count; do
      printf '| %s | %.2f%% | %s/%s |\n' "$file" "$percent" "$covered" "$count"
    done <<< "$summary"
  } >> "$GITHUB_STEP_SUMMARY"
fi

failures="$(jq -r \
  --arg workspace_root "$workspace_root" \
  --arg source_root "$source_root" \
  --argjson minimum "$minimum" '
    .data[].files[]
    | select(.filename | startswith($source_root))
    | select(.filename | contains("/src/"))
    | select(.filename | endswith("/test.rs") | not)
    | select(.filename | endswith("/tests.rs") | not)
    | select(.summary.lines.count > 0)
    | select(.summary.lines.percent < $minimum)
    | "\(.filename | ltrimstr($workspace_root)): \(.summary.lines.percent)%"
  ' "$report")"

if [[ -n "$failures" ]]; then
  printf '\nFiles below %s%% line coverage:\n%s\n' "$minimum" "$failures" >&2
  exit 1
fi
