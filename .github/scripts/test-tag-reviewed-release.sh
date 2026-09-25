#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

git init -q --bare "$scratch/remote.git"
git init -q -b main "$scratch/work"
git -C "$scratch/work" config user.name "Release test"
git -C "$scratch/work" config user.email "release-test@example.invalid"
printf 'first reviewed commit\n' > "$scratch/work/source.txt"
git -C "$scratch/work" add source.txt
git -C "$scratch/work" commit -qm 'Initial reviewed source'
git -C "$scratch/work" remote add origin "$scratch/remote.git"
git -C "$scratch/work" push -q -u origin main

mkdir "$scratch/fakebin"
cat > "$scratch/fakebin/cargo" <<'FAKE_CARGO'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == metadata ]]
printf '{"packages":[{"name":"tinybrowser","version":"%s"}]}\n' "$TEST_VERSION"
FAKE_CARGO
chmod +x "$scratch/fakebin/cargo"

run_tag() {
  (
    cd "$scratch/work"
    PATH="$scratch/fakebin:$PATH" \
      TEST_VERSION="${3:-0.2.2}" EXPECTED_VERSION="$1" \
      GITHUB_REF="${2:-refs/heads/main}" \
      GITHUB_OUTPUT="$scratch/outputs" RELEASE_PACKAGE=tinybrowser \
      "$script_dir/tag-reviewed-release.sh"
  )
}

if run_tag 0.2.3 > "$scratch/mismatch.out" 2>&1; then
  echo "tagged a version that was not merged" >&2
  exit 1
fi
grep -q 'expected 0.2.3' "$scratch/mismatch.out"
if run_tag 0.2.2 refs/heads/feature > "$scratch/branch.out" 2>&1; then
  echo "tagged a non-main branch" >&2
  exit 1
fi
grep -q 'only allowed from main' "$scratch/branch.out"

printf 'unreviewed local commit\n' >> "$scratch/work/source.txt"
git -C "$scratch/work" commit -qam 'Local unreviewed change'
if run_tag 0.2.2 > "$scratch/unreviewed.out" 2>&1; then
  echo "tagged a commit not on protected main" >&2
  exit 1
fi
grep -q 'not the current protected main commit' "$scratch/unreviewed.out"

git -C "$scratch/work" push -q origin main
reviewed_sha="$(git -C "$scratch/work" rev-parse HEAD)"
run_tag 0.2.2 > "$scratch/created.out"
[[ "$(git -C "$scratch/work" rev-list -n 1 v0.2.2)" == "$reviewed_sha" ]]
[[ "$(git -C "$scratch/work" ls-remote origin refs/heads/main | awk '{print $1}')" == "$reviewed_sha" ]]
[[ -n "$(git -C "$scratch/work" ls-remote origin refs/tags/v0.2.2)" ]]
grep -q '^next_version=0.2.2$' "$scratch/outputs"
grep -q '^tag=v0.2.2$' "$scratch/outputs"

run_tag 0.2.2 > "$scratch/existing.out"
grep -q 'using existing tag v0.2.2' "$scratch/existing.out"

git -C "$scratch/work" update-ref refs/tags/v0.2.3 HEAD
git -C "$scratch/work" push -q origin refs/tags/v0.2.3
if run_tag 0.2.3 refs/heads/main 0.2.3 > "$scratch/lightweight.out" 2>&1; then
  echo "accepted a lightweight release tag" >&2
  exit 1
fi
grep -q 'must be annotated' "$scratch/lightweight.out"

printf 'later reviewed commit\n' >> "$scratch/work/source.txt"
git -C "$scratch/work" commit -qam 'Later reviewed source'
git -C "$scratch/work" push -q origin main
[[ "$(git -C "$scratch/work" ls-remote origin refs/heads/main | awk '{print $1}')" == \
   "$(git -C "$scratch/work" rev-parse HEAD)" ]]
if run_tag 0.2.2 > "$scratch/stale-tag.out" 2>&1; then
  echo "accepted a tag pointing to an older commit" >&2
  exit 1
fi
grep -q 'not reviewed main' "$scratch/stale-tag.out"

echo "reviewed release tag tests passed"
