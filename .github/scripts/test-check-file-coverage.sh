#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "$script_dir/../.." && pwd -P)"
scratch="$(mktemp -d)"
scratch="$(cd "$scratch" && pwd -P)"
trap 'rm -rf "$scratch"' EXIT

python3 - "$scratch" <<'PY' > "$scratch/good.json"
import json
import sys

root = sys.argv[1]

def file(path, covered, count):
    return {
        "filename": f"{root}/{path}",
        "summary": {"lines": {"covered": covered, "count": count,
                              "percent": covered / count * 100}},
    }

print(json.dumps({"data": [{"files": [
    file("crates/example/src/ops.rs", 9, 10),
    file("crates/example/src/test.rs", 0, 10),
    file("crates/example/tests/integration.rs", 0, 10),
    file("vendor/tinybus/crates/tinybus/src/lib.rs", 0, 10),
]}]}))
PY

python3 - "$scratch" <<'PY' > "$scratch/bad.json"
import json
import sys

root = sys.argv[1]
print(json.dumps({"data": [{"files": [{
    "filename": f"{root}/crates/example/src/ops.rs",
    "summary": {"lines": {"covered": 8, "count": 10, "percent": 80}},
}]}]}))
PY

python3 - "$scratch" <<'PY' > "$scratch/test-only.json"
import json
import sys

root = sys.argv[1]
print(json.dumps({"data": [{"files": [{
    "filename": f"{root}/crates/example/src/test.rs",
    "summary": {"lines": {"covered": 10, "count": 10, "percent": 100}},
}]}]}))
PY

(
  cd "$scratch"
  "$script_dir/check-file-coverage-report.sh" 90 good.json "$scratch" > good.out
  grep -q 'crates/example/src/ops.rs' good.out
  if grep -Eq 'test.rs|integration.rs|vendor/' good.out; then
    echo "test-only or vendored file was counted" >&2
    exit 1
  fi
  if "$script_dir/check-file-coverage-report.sh" 90 bad.json "$scratch" > bad.out 2>&1; then
    echo "production file below 90% passed" >&2
    exit 1
  fi
  grep -q 'crates/example/src/ops.rs: 80%' bad.out
  if "$script_dir/check-file-coverage-report.sh" 90 test-only.json "$scratch" > empty.out 2>&1; then
    echo "report with no production files passed" >&2
    exit 1
  fi
  grep -q 'no production files' empty.out
)

mkdir "$scratch/fakebin"
cat > "$scratch/fakebin/cargo" <<'FAKE_CARGO'
#!/usr/bin/env bash
set -euo pipefail
[[ "${TINYBROWSER_LIVE_TESTS:-}" == 1 ]] || {
  echo "coverage did not opt in to live browser tests" >&2
  exit 1
}
[[ "$1" == llvm-cov ]]
shift
for flag in --locked --workspace --all-targets --all-features --json; do
  [[ " $* " == *" $flag "* ]] || exit 1
done
while [[ "$1" != --output-path ]]; do shift; done
cp "$COVERAGE_FIXTURE" "$2"
FAKE_CARGO
chmod +x "$scratch/fakebin/cargo"
python3 - "$scratch/good.json" "$scratch" "$repo_root" <<'PY' > "$scratch/wrapper-good.json"
import json
import sys

report = json.load(open(sys.argv[1]))
for group in report["data"]:
    for file in group["files"]:
        file["filename"] = file["filename"].replace(sys.argv[2] + "/", sys.argv[3] + "/", 1)
print(json.dumps(report))
PY
(
  cd "$repo_root"
  TINYBROWSER_LIVE_TESTS=1 PATH="$scratch/fakebin:$PATH" \
    COVERAGE_FIXTURE="$scratch/wrapper-good.json" \
    "$script_dir/check-file-coverage.sh" 90 "$scratch/wrapped.json" > "$scratch/wrapped.out"
  grep -q 'crates/example/src/ops.rs' "$scratch/wrapped.out"
  if PATH="$scratch/fakebin:$PATH" COVERAGE_FIXTURE="$scratch/wrapper-good.json" \
      "$script_dir/check-file-coverage.sh" 90 "$scratch/not-opted-in.json" > "$scratch/not-opted-in.out" 2>&1; then
    echo "coverage gate accepted missing live-test opt-in" >&2
    exit 1
  fi
  grep -q 'requires TINYBROWSER_LIVE_TESTS=1' "$scratch/not-opted-in.out"
  if TINYBROWSER_CHROME="$scratch/missing-chrome" \
      TINYBROWSER_LIVE_TESTS=1 PATH="$scratch/fakebin:$PATH" \
      COVERAGE_FIXTURE="$scratch/wrapper-good.json" \
      "$script_dir/check-file-coverage.sh" 90 "$scratch/missing.json" > "$scratch/missing.out" 2>&1; then
    echo "missing configured Chrome passed" >&2
    exit 1
  fi
  grep -q 'not executable' "$scratch/missing.out"
)
if (
  cd "$scratch"
  TINYBROWSER_LIVE_TESTS=1 PATH="$scratch/fakebin:$PATH" \
    COVERAGE_FIXTURE="$scratch/wrapper-good.json" \
    "$script_dir/check-file-coverage.sh" 90 "$scratch/wrong-directory.json" > "$scratch/wrong-directory.out" 2>&1
); then
  echo "coverage gate accepted a different working directory" >&2
  exit 1
fi
grep -q 'run the coverage gate from the repository root' "$scratch/wrong-directory.out"

echo "coverage script tests passed"
