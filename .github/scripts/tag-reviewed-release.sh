#!/usr/bin/env bash
set -euo pipefail

: "${EXPECTED_VERSION:?set EXPECTED_VERSION to the merged release version}"
: "${GITHUB_OUTPUT:?set GITHUB_OUTPUT for release job outputs}"
: "${RELEASE_PACKAGE:=tinybrowser}"

if [[ ! "$EXPECTED_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "release version must be major.minor.patch: $EXPECTED_VERSION" >&2
  exit 1
fi
if [[ "${GITHUB_REF:-}" != refs/heads/main ]]; then
  echo "release tagging is only allowed from main" >&2
  exit 1
fi
root="$(git rev-parse --show-toplevel)"
if [[ "$(pwd -P)" != "$(cd "$root" && pwd -P)" ]]; then
  echo "run release tagging from the repository root" >&2
  exit 1
fi
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "release checkout has uncommitted tracked changes" >&2
  exit 1
fi

metadata="$(cargo metadata --format-version 1 --no-deps --locked)"
crate_name="$(jq -r --arg name "$RELEASE_PACKAGE" \
  '.packages[] | select(.name == $name) | .name' <<< "$metadata")"
current_version="$(jq -r --arg name "$RELEASE_PACKAGE" \
  '.packages[] | select(.name == $name) | .version' <<< "$metadata")"
if [[ "$crate_name" != "$RELEASE_PACKAGE" || "$current_version" != "$EXPECTED_VERSION" ]]; then
  echo "merged $RELEASE_PACKAGE version is $current_version; expected $EXPECTED_VERSION" >&2
  exit 1
fi

head_sha="$(git rev-parse HEAD)"
main_sha="$(git ls-remote origin refs/heads/main | awk '{print $1}')"
if [[ -z "$main_sha" || "$head_sha" != "$main_sha" ]]; then
  echo "release HEAD is not the current protected main commit" >&2
  exit 1
fi

tag="v${current_version}"
git fetch --tags origin
if git rev-parse --verify --quiet "refs/tags/${tag}" >/dev/null; then
  if [[ "$(git cat-file -t "refs/tags/${tag}")" != tag ]]; then
    echo "existing release tag $tag must be annotated" >&2
    exit 1
  fi
  tagged_sha="$(git rev-list -n 1 "$tag")"
  if [[ "$tagged_sha" != "$head_sha" ]]; then
    echo "existing tag $tag points to $tagged_sha, not reviewed main $head_sha" >&2
    exit 1
  fi
  echo "using existing tag $tag on reviewed main $head_sha"
else
  git config user.name "github-actions[bot]"
  git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
  git tag -a "$tag" -m "Release $tag"
  git push origin "refs/tags/${tag}"
  echo "created tag $tag on reviewed main $head_sha"
fi

{
  echo "crate_name=$crate_name"
  echo "next_version=$current_version"
  echo "tag=$tag"
} >> "$GITHUB_OUTPUT"
