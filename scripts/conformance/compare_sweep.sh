#!/bin/bash
# Rerun finished GitHub runs locally with `wrkflw compare`, and tabulate how
# many jobs agree. Each repository is cloned once into a cache directory
# that outlives the run (temporary directories get cleaned), checked out
# detached at the commit the run tested, and compared there.
#
# Usage: scripts/conformance/compare_sweep.sh [compare_runs.txt] [wrkflw binary]
# Needs: git, gh (signed in), jq, and Docker or Podman. Prints one
# tab-separated line per run:
#   repo  run  agree  agreed  differed  not_compared  seconds
# from compare's own JSON report, never from the jobs' output. Exits 1 if
# any run disagrees. Each run's full report and log go to
# $WRKFLW_SWEEP_DIR/logs.
#
# This runs the repositories' CI on this machine: only list ones you trust.

set -uo pipefail

RUNS="${1:-$(dirname "$0")/compare_runs.txt}"
WRKFLW="${2:-wrkflw}"
if [[ "$WRKFLW" == */* ]]; then
  WRKFLW="$(cd "$(dirname "$WRKFLW")" && pwd)/$(basename "$WRKFLW")"
fi
SWEEP_DIR="${WRKFLW_SWEEP_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/wrkflw-sweep}"
mkdir -p "$SWEEP_DIR/repos" "$SWEEP_DIR/logs"

for tool in git gh jq; do
  command -v "$tool" >/dev/null || { echo "$tool is required" >&2; exit 2; }
done

disagreed=0
printf 'repo\trun\tagree\tagreed\tdiffered\tnot_compared\tseconds\n'

# The list is read on fd 3, so a command reading stdin can't eat lines of it.
while read -r repo run extra <&3; do
  [[ -z "$repo" || "$repo" == \#* ]] && continue
  name="${repo//\//_}"
  dir="$SWEEP_DIR/repos/$name"
  log="$SWEEP_DIR/logs/$name-$run"
  sha=$(gh api "repos/$repo/actions/runs/$run" --jq .head_sha 2>/dev/null) || {
    printf '%s\t%s\tn/a: cannot read the run\t-\t-\t-\t-\n' "$repo" "$run"; continue; }
  if [[ ! -d "$dir/.git" ]]; then
    git clone -q "https://github.com/$repo.git" "$dir" || {
      printf '%s\t%s\tn/a: cannot clone\t-\t-\t-\t-\n' "$repo" "$run"; continue; }
  fi
  if ! git -C "$dir" cat-file -e "$sha^{commit}" 2>/dev/null; then
    git -C "$dir" fetch -q origin "$sha" 2>/dev/null || git -C "$dir" fetch -q origin
  fi
  git -C "$dir" checkout -q --detach "$sha" || {
    printf '%s\t%s\tn/a: cannot check out %s\t-\t-\t-\t-\n' "$repo" "$run" "$sha"; continue; }

  # Variables listed after the run id, such as WRKFLW_DOCKER_IN_JOBS=1.
  env_args=()
  for pair in $extra; do env_args+=("$pair"); done

  start=$(date +%s)
  (cd "$dir" && env ${env_args[@]+"${env_args[@]}"} WRKFLW_NO_HISTORY=1 \
      "$WRKFLW" compare "https://github.com/$repo/actions/runs/$run" --format json \
      >"$log.json" 2>"$log.log") </dev/null
  seconds=$(( $(date +%s) - start ))

  report=$(jq -c 'select(.type == "compare")' "$log.json" 2>/dev/null | tail -n 1)
  if [[ -z "$report" ]]; then
    printf '%s\t%s\tn/a: no report (see %s.log)\t-\t-\t-\t%s\n' "$repo" "$run" "$log" "$seconds"
    disagreed=1
    continue
  fi
  read -r agree agreed differed unknown < <(jq -r '[.agree,
      ([.jobs[] | select(.agree == true)] | length),
      ([.jobs[] | select(.agree == false)] | length),
      ([.jobs[] | select(.agree == null)] | length)] | @tsv' <<<"$report")
  [[ "$agree" == "true" ]] || disagreed=1
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$repo" "$run" "$agree" "$agreed" "$differed" "$unknown" "$seconds"
done 3<"$RUNS"

exit "$disagreed"
