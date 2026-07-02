#!/usr/bin/env bash
# release-cut.sh — user-approved release cut for a merged release-please PR.
#
# Why this exists: the repository GITHUB_TOKEN cannot create releases/tags that
# trigger workflows, so merging the release-please PR leaves the version untagged
# (the release workflow warns: "recover this with a user-approved release cut").
# This script IS that cut: it finds the newest merged 'autorelease: pending' PR,
# tags its merge commit with the user's credentials, pushes the tag (which
# triggers the tag build: binaries + sigstore + SBOM + crates.io publish), and
# verifies the result end to end.
#
# Usage: scripts/release-cut.sh [--yes]     (or: just release-cut)

set -euo pipefail

YES=false
[[ "${1:-}" == "--yes" ]] && YES=true

repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"

pending="$(
  gh pr list --repo "${repo}" \
    --state merged --label 'autorelease: pending' --limit 20 \
    --json number,title,mergedAt,mergeCommit \
    --jq 'sort_by(.mergedAt) | reverse | .[0] // empty'
)"
if [[ -z "${pending}" ]]; then
  echo "No merged 'autorelease: pending' PR found — nothing to cut."
  exit 0
fi

pr_number="$(jq -r '.number' <<<"${pending}")"
pr_title="$(jq -r '.title' <<<"${pending}")"
pr_sha="$(jq -r '.mergeCommit.oid // empty' <<<"${pending}")"
version="$(sed -nE 's/^chore: release[[:space:]]+([0-9]+\.[0-9]+\.[0-9]+).*$/\1/p' <<<"${pr_title}")"

if [[ -z "${pr_sha}" || -z "${version}" ]]; then
  echo "error: cannot derive sha/version from PR #${pr_number} ('${pr_title}')" >&2
  exit 1
fi
tag="v${version}"

if git ls-remote --exit-code --tags origin "refs/tags/${tag}" >/dev/null 2>&1; then
  echo "Tag ${tag} already exists on origin — nothing to cut."
  exit 0
fi

echo "Release cut: ${tag} at ${pr_sha} (PR #${pr_number})"
if ! ${YES}; then
  printf "Proceed? [y/N] "
  read -r ans
  [[ "${ans}" == "y" || "${ans}" == "Y" ]] || { echo "aborted"; exit 1; }
fi

git fetch origin >/dev/null
git tag -a "${tag}" "${pr_sha}" -m "galdr ${version}"
git push origin "${tag}"
echo "Tag pushed — waiting for the release workflow..."

run_id=""
for _ in $(seq 1 12); do
  sleep 5
  run_id="$(gh run list --repo "${repo}" --event push --branch "${tag}" \
    --json databaseId --jq '.[0].databaseId // empty' 2>/dev/null || true)"
  [[ -n "${run_id}" ]] && break
done
if [[ -z "${run_id}" ]]; then
  echo "warn: could not find the tag workflow run; check: gh run list --repo ${repo}" >&2
  exit 1
fi

gh run watch "${run_id}" --repo "${repo}" --exit-status
echo "Workflow done. Verifying:"
gh release view "${tag}" --repo "${repo}" --json name,publishedAt --jq '"  release: " + .name + " @ " + .publishedAt'

for _ in $(seq 1 18); do
  latest="$(curl -s --max-time 5 https://index.crates.io/ga/ld/galdr | tail -1 | jq -r '.vers' 2>/dev/null || true)"
  if [[ "${latest}" == "${version}" ]]; then
    echo "  crates.io: ${latest} ✓"
    exit 0
  fi
  sleep 10
done
echo "warn: crates.io index not showing ${version} yet (propagation can lag; re-check later)" >&2
