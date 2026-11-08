#!/bin/bash
# Compare the jobs `wrkflw plan` expects with the jobs GitHub ran, across
# real repositories. Nothing is executed: workflow files are fetched at the
# commit GitHub ran and only planned.
#
# Usage: scripts/conformance/plan_corpus.sh [repos.txt] [wrkflw binary]
# Needs: gh (signed in), jq. Prints one tab-separated line per workflow run:
#   repo  workflow  github_jobs  planned_jobs  result
# where result is `match`, `mismatch`, or `n/a: <why>`. Exits 1 on any
# mismatch, so it can run on a schedule.

set -uo pipefail

REPOS="${1:-$(dirname "$0")/repos.txt}"
WRKFLW="${2:-wrkflw}"
# The script plans from inside a scratch directory, so a relative path
# to the binary has to be made absolute first.
if [[ "$WRKFLW" == */* ]]; then
  WRKFLW="$(cd "$(dirname "$WRKFLW")" && pwd)/$(basename "$WRKFLW")"
fi
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

command -v gh >/dev/null || { echo "gh is required" >&2; exit 2; }
command -v jq >/dev/null || { echo "jq is required" >&2; exit 2; }

mismatches=0
printf 'repo\tworkflow\tgithub_jobs\tplanned_jobs\tresult\n'

while read -r repo; do
  [[ -z "$repo" || "$repo" == \#* ]] && continue
  # The latest push or pull_request run of each workflow that passed or failed.
  runs=$(gh api "repos/$repo/actions/runs?per_page=50&status=completed" \
      --jq '[.workflow_runs[] | select((.event=="push" or .event=="pull_request")
                                         and (.conclusion=="success" or .conclusion=="failure"))]
            | unique_by(.path) | .[] | [.id, .path, .head_sha] | @tsv' 2>/dev/null) || {
    printf '%s\t-\t-\t-\tn/a: cannot list runs\n' "$repo"; continue; }
  while IFS=$'\t' read -r id path sha; do
    [[ -z "$id" ]] && continue
    path="${path%%@*}"
    jobs=$(gh api "repos/$repo/actions/runs/$id/jobs?per_page=100" 2>/dev/null) || {
      printf '%s\t%s\t-\t-\tn/a: cannot list jobs\n' "$repo" "$path"; continue; }
    github=$(jq '.total_count' <<<"$jobs")
    odd=$(jq '[.jobs[] | select(.conclusion=="skipped" or .conclusion=="cancelled")] | length' <<<"$jobs")
    called=$(jq '[.jobs[] | select(.name | contains(" / "))] | length' <<<"$jobs")
    dir="$WORK/$repo"
    mkdir -p "$dir/$(dirname "$path")"
    gh api "repos/$repo/contents/$path?ref=$sha" --jq '.content' 2>/dev/null \
      | base64 --decode > "$dir/$path" 2>/dev/null || {
      printf '%s\t%s\t%s\t-\tn/a: cannot fetch the file\n' "$repo" "$path" "$github"; continue; }
    plan=$(cd "$dir" && "$WRKFLW" --format json plan "$path" 2>/dev/null)
    if [[ -z "$plan" ]] || [[ "$(jq -r '.type' <<<"$plan")" != "plan" ]]; then
      printf '%s\t%s\t%s\t-\tmismatch: wrkflw cannot plan it: %s\n' "$repo" "$path" "$github" \
        "$(jq -r '.message // "no output"' <<<"$plan" 2>/dev/null | tr '\n\t' '  ' | cut -c1-160)"
      mismatches=$((mismatches + 1)); continue
    fi
    dynamic=$(jq '[.jobs[] | select(.matrix.dynamic == true)] | length' <<<"$plan")
    planned=$(jq '[.jobs[] | (.matrix.combinations // [1] | length)] | add // 0' <<<"$plan")
    if (( github == 0 )); then result="n/a: GitHub ran no job (awaiting approval?)"
    elif (( called > 0 )); then result="n/a: calls another workflow"
    elif (( odd > 0 )); then result="n/a: GitHub skipped or cancelled jobs"
    elif (( dynamic > 0 )); then result="n/a: matrix computed at run time"
    elif (( planned == github )); then result="match"
    else result="mismatch"; mismatches=$((mismatches + 1)); fi
    printf '%s\t%s\t%s\t%s\t%s\n' "$repo" "$path" "$github" "$planned" "$result"
  done <<<"$runs"
done < "$REPOS"

(( mismatches == 0 ))
