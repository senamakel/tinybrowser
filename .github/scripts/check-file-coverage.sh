#!/usr/bin/env bash
set -euo pipefail

minimum="${1:-90}"
report="${2:-coverage.json}"

if [[ -n "${TINYBROWSER_CHROME:-}" && ! -x "$TINYBROWSER_CHROME" ]]; then
  echo "configured TINYBROWSER_CHROME is not executable" >&2
  exit 1
fi

# The Chrome suites serve their pages on loopback and fail if Chrome is absent.
# Without this opt-in their tests silently skip, leaving browser paths out of
# the coverage report even though they are exercised in the release product.
TINYBROWSER_LIVE_TESTS=1 cargo llvm-cov \
  --locked \
  --workspace \
  --all-targets \
  --all-features \
  --json \
  --output-path "$report"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
"$script_dir/check-file-coverage-report.sh" "$minimum" "$report"
