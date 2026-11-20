#!/bin/bash
# Rerun finished GitHub runs locally with `wrkflw compare`, and tabulate how
# many jobs agree. Each repository is cloned once into a cache directory
# that outlives the run (temporary directories get cleaned), checked out
# detached at the commit the run tested, and compared there.
#
# Usage: scripts/conformance/compare_sweep.sh [compare_runs.txt] [wrkflw binary]
# Needs: git, gh (signed in), jq, and Docker or Podman. Prints one
# tab-separated line per run:
#   repo  run  agree  agreed  differed  not_compared  unverified  built  seconds
#   local_s  github_s  change
# from compare's own JSON report, never from the jobs' output. `unverified`
# counts agreeing jobs that skipped an action GitHub ran or ran as Linux for
# a macOS job. `seconds` is the whole compare, fetching the run included;
# `local_s` the local run alone and `github_s` the GitHub run, from its
# start to its last update (a re-run counts from the start of its latest
# attempt, as GitHub resets `run_started_at`). `built` counts the
# job images built during the run: 0 means its time is a warm one, so only
# compare times of runs with the same `built`. Each run's full report and
# log go to $WRKFLW_SWEEP_DIR/logs.
#
# `change` compares the run with its line in the expectations beside the
# list of runs: <name>_expected.tsv for a list named <name>_runs.txt, so
# compare_runs.txt is checked against compare_expected.tsv and
# held_out_runs.txt against held_out_expected.tsv (or $WRKFLW_SWEEP_EXPECTED): `same`, `better`, `WORSE`
# when fewer jobs agree or more differ, or `new` for a run not listed
# there; more `unverified` than recorded counts as WORSE too, where the
# line records it. Some runs are known to differ, so the sweep exits 1 only when a
# run is WORSE or gives no report, not whenever one disagrees. With
# WRKFLW_SWEEP_RECORD=1 the results are written to that file instead, as
# the new expectation: do that after a change that is meant to move them.
#
# This runs the repositories' CI on this machine: only list ones you trust.

set -uo pipefail

RUNS="${1:-$(dirname "$0")/compare_runs.txt}"
# Each list of runs keeps its own expectations, so recording one sweep
# never overwrites another's.
case "$(basename "$RUNS")" in
  *_runs.txt) default_expected="${RUNS%_runs.txt}_expected.tsv" ;;
  *) default_expected="$(dirname "$RUNS")/compare_expected.tsv" ;;
esac
EXPECTED="${WRKFLW_SWEEP_EXPECTED:-$default_expected}"
RECORD="${WRKFLW_SWEEP_RECORD:-}"
WRKFLW="${2:-wrkflw}"
if [[ "$WRKFLW" == */* ]]; then
  WRKFLW="$(cd "$(dirname "$WRKFLW")" && pwd)/$(basename "$WRKFLW")"
fi
SWEEP_DIR="${WRKFLW_SWEEP_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/wrkflw-sweep}"
mkdir -p "$SWEEP_DIR/repos" "$SWEEP_DIR/logs"

for tool in git gh jq; do
  command -v "$tool" >/dev/null || { echo "$tool is required" >&2; exit 2; }
done

# How many job images the engine holds, or nothing without an engine CLI.
# The GitHub CLI's layer on a job image is an image of its own, tagged
# `<tag>-gh-<hash>`: it counts with the image it is on, not as a second.
engine=""
for cli in docker podman; do
  if command -v "$cli" >/dev/null && "$cli" info >/dev/null 2>&1; then engine="$cli"; break; fi
done
job_images() {
  [[ -n "$engine" ]] && "$engine" images --format '{{.Tag}}' --filter 'reference=wrkflw-combined' 2>/dev/null |
    sed 's/-gh-[0-9a-f]*$//' | sort -u | wc -l | tr -d ' '
}

# What compare_expected.tsv says of a run: "agreed differed unverified",
# with "-" for unverified on a line recorded before it was, or nothing.
expected_for() {
  [[ -f "$EXPECTED" ]] || return 0
  awk -F'\t' -v repo="$1" -v run="$2" \
    '$1 == repo && $2 == run { print $3, $4, ($6 == "" ? "-" : $6) }' "$EXPECTED"
}

# One compare, with its report in $log.json. A GitHub API error (5xx, or a
# rate limit that passes) is tried again: it says nothing about the run.
compare_once() {
  (cd "$dir" && env ${env_args[@]+"${env_args[@]}"} WRKFLW_NO_HISTORY=1 \
      "$WRKFLW" compare "https://github.com/$repo/actions/runs/$run" --format json \
      >"$log.json" 2>"$log.log") </dev/null
}
fetch_failed() {
  jq -e 'select(.type == "error" and .kind == "fetch_failed")' "$log.json" >/dev/null 2>&1
}

failed=0
missing=0
recorded=""
printf 'repo\trun\tagree\tagreed\tdiffered\tnot_compared\tunverified\tbuilt\tseconds\tlocal_s\tgithub_s\tchange\n'
no_result() {  # repo run reason [seconds]
  printf '%s\t%s\tn/a: %s\t-\t-\t-\t-\t-\t%s\t-\t-\t-\n' "$1" "$2" "$3" "${4:--}"
  failed=1
  missing=1
}

# The list is read on fd 3, so a command reading stdin can't eat lines of it.
while read -r repo run extra <&3; do
  [[ -z "$repo" || "$repo" == \#* ]] && continue
  name="${repo//\//_}"
  dir="$SWEEP_DIR/repos/$name"
  log="$SWEEP_DIR/logs/$name-$run"
  sha="" github_s="-"
  for attempt in 1 2 3; do
    # The commit, and how long the run took on GitHub, from the start of
    # its latest attempt to its last update.
    if run_info=$(gh api "repos/$repo/actions/runs/$run" \
        --jq '[.head_sha, ((.updated_at | fromdate) - (.run_started_at | fromdate))] | @tsv' \
        2>/dev/null); then
      read -r sha github_s <<<"$run_info"
      break
    fi
    sha=""
    (( attempt < 3 )) && sleep $((attempt * 5))
  done
  [[ -n "$sha" ]] || { no_result "$repo" "$run" "cannot read the run"; continue; }
  if [[ ! -d "$dir/.git" ]]; then
    git clone -q "https://github.com/$repo.git" "$dir" || {
      no_result "$repo" "$run" "cannot clone"; continue; }
  fi
  if ! git -C "$dir" cat-file -e "$sha^{commit}" 2>/dev/null; then
    git -C "$dir" fetch -q origin "$sha" 2>/dev/null || git -C "$dir" fetch -q origin
  fi
  git -C "$dir" checkout -q --detach "$sha" || {
    no_result "$repo" "$run" "cannot check out $sha"; continue; }

  # Variables listed after the run id, such as WRKFLW_DOCKER_IN_JOBS=1. A
  # word that isn't NAME=value would be run by `env` as a command.
  env_args=()
  bad=""
  for pair in $extra; do
    if [[ "$pair" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; then
      env_args+=("$pair")
    else
      bad="$pair"
    fi
  done
  if [[ -n "$bad" ]]; then
    no_result "$repo" "$run" "not NAME=value: $bad"
    continue
  fi

  images_before=$(job_images)
  start=$(date +%s)
  for attempt in 1 2 3; do
    compare_once
    fetch_failed || break
    (( attempt < 3 )) && sleep $((attempt * 10))
  done
  seconds=$(( $(date +%s) - start ))
  images_after=$(job_images)
  built="-"
  [[ -n "$images_before" && -n "$images_after" ]] && built=$(( images_after - images_before ))

  report=$(jq -c 'select(.type == "compare")' "$log.json" 2>/dev/null | tail -n 1)
  if [[ -z "$report" ]]; then
    no_result "$repo" "$run" "no report (see $log.log)" "$seconds"
    continue
  fi
  read -r agree agreed differed unknown unverified local_s < <(jq -r '[.agree,
      ([.jobs[] | select(.agree == true)] | length),
      ([.jobs[] | select(.agree == false)] | length),
      ([.jobs[] | select(.agree == null)] | length),
      (.unverified_agreements // 0),
      (if .local.duration_ms then (.local.duration_ms / 1000 | round) else "-" end)] | @tsv' <<<"$report")

  change="new"
  want_agreed="" want_differed="" want_unverified=""
  read -r want_agreed want_differed want_unverified < <(expected_for "$repo" "$run")
  if [[ -n "${want_agreed:-}" && -n "${want_differed:-}" ]]; then
    more_unverified=0 fewer_unverified=0
    if [[ "${want_unverified:--}" != "-" ]]; then
      (( unverified > want_unverified )) && more_unverified=1
      (( unverified < want_unverified )) && fewer_unverified=1
    fi
    if (( agreed < want_agreed || differed > want_differed || more_unverified )); then
      change="WORSE"
      failed=1
    elif (( agreed > want_agreed || differed < want_differed || fewer_unverified )); then
      change="better"
    else
      change="same"
    fi
  fi
  recorded+="$repo"$'\t'"$run"$'\t'"$agreed"$'\t'"$differed"$'\t'"$unknown"$'\t'"$unverified"$'\n'
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$repo" "$run" "$agree" "$agreed" "$differed" "$unknown" "$unverified" "$built" "$seconds" \
    "$local_s" "$github_s" "$change"
done 3<"$RUNS"

if [[ -n "$RECORD" && "$RECORD" != 0 ]]; then
  # A run with no report has nothing to record, and would drop its line.
  if (( missing )); then
    echo "not recorded: a run gave no report" >&2
  else
    {
      printf '# What compare_sweep.sh expects of each run: fewer agreeing, or more\n'
      printf '# differing or unverified jobs, than this is a regression. Written by\n'
      printf '# WRKFLW_SWEEP_RECORD=1; say in the commit why a line changed.\n'
      printf '# repo\trun\tagreed\tdiffered\tnot_compared\tunverified\n'
      printf '%s' "$recorded"
    } >"$EXPECTED" || { echo "cannot write $EXPECTED" >&2; exit 2; }
    echo "recorded $EXPECTED" >&2
    exit 0
  fi
fi

exit "$failed"
