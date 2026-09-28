#!/usr/bin/env bash
# Both release entry points use the tagged commit, including annotated tags.
set -euo pipefail

tag="${1:?usage: advance-release-branch.sh fvid-vVERSION}"
[[ "$tag" == fvid-v* ]] || { echo "Expected a fvid-v tag" >&2; exit 1; }
git check-ref-format "refs/tags/$tag"
commit="$(git rev-parse --verify "refs/tags/${tag}^{commit}")"
# An ordinary push creates the branch or advances it; an older/divergent tag
# cannot roll the release branch back. Repeating the same release is harmless.
git push origin "$commit:refs/heads/release"
echo "release -> $commit ($tag)"
