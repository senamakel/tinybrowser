#!/usr/bin/env bash
set -euo pipefail

minimum="${1:-90}"
report="${2:-coverage.json}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
workspace_root="$(cd "$script_dir/../.." && pwd -P)"
if [[ "$(pwd -P)" != "$workspace_root" ]]; then
  echo "run the coverage gate from the repository root: $workspace_root" >&2
  exit 1
fi

if [[ "${TINYBROWSER_LIVE_TESTS:-}" != 1 ]]; then
  echo "coverage gate requires TINYBROWSER_LIVE_TESTS=1 and a usable Chrome" >&2
  exit 1
fi
if [[ -n "${TINYBROWSER_CHROME:-}" && ! -x "$TINYBROWSER_CHROME" ]]; then
  echo "configured TINYBROWSER_CHROME is not executable" >&2
  exit 1
fi

# The Chrome suites serve their pages on loopback and fail if Chrome is absent.
# Requiring explicit opt-in prevents an apparently green coverage run whose
# browser tests silently skipped on a developer machine without Chrome.
cargo llvm-cov \
  --locked \
  --workspace \
  --all-targets \
  --all-features \
  --json \
  --output-path "$report"

"$script_dir/check-file-coverage-report.sh" "$minimum" "$report" "$workspace_root"
