#!/usr/bin/env bash
# Reports PSS/RSS/USS/SwapPss memory for Linh's process tree (the `linh`
# process plus its WebKitWebProcess/WebKitNetworkProcess/GPU descendants),
# read from each process's /proc/<pid>/smaps_rollup. See docs/profiling.md
# (or README.md's "Profiling memory" section) for how to read these
# numbers and why PSS, not RSS, is the number to compare across snapshots.
set -euo pipefail
# Without this, a failing command substitution (e.g. `x="$(err ...)"`,
# where `err` calls `exit`) only kills that subshell — `set -e` does not
# propagate the failure to the enclosing function/script by default.
shopt -s inherit_errexit

PROG_NAME="$(basename "$0")"
DEFAULT_EXE="/usr/bin/linh"
DEFAULT_SETTLE=5

err() {
  echo "${PROG_NAME}: error: $*" >&2
  exit 1
}

usage() {
  cat <<EOF
Usage:
  ${PROG_NAME} snapshot <label> [--pid PID] [--exe PATH] [--out FILE]
  ${PROG_NAME} guided [--pid PID] [--exe PATH] [--out FILE] [--settle SECONDS]

Reports Linh's process tree memory: RSS, PSS, Pss_Anon, Pss_File, USS
(Private_Clean+Private_Dirty) and SwapPss per process, plus a TREE TOTAL.
PSS is the headline number for comparing snapshots — RSS is never summed
across the tree, since shared pages (WebKit's shared library/cache
mappings) would be double-counted.

  --pid PID    use this pid as the Linh root instead of auto-discovering
               it by exe path
  --exe PATH   match this exe path when auto-discovering the root pid
               (default: ${DEFAULT_EXE}) — use for a dev build
  --out FILE   also append one TSV summary row per snapshot to FILE
               (header is written once, if FILE does not already exist)
  --settle N   guided mode only: seconds to wait after Enter before
               snapshotting, to let the window actually settle
               (default: ${DEFAULT_SETTLE})

snapshot <label>:
  Prints one labeled table for the current state of the tree.

guided:
  Walks four states — (a) idle in tray, (b) Settings open, (c) prompt
  shown, (d) everything closed again — prompting you to set up each one,
  then snapshotting it, and ends with a summary table of PSS deltas
  against (a).
EOF
}

# --- process discovery -----------------------------------------------

# True (0) if $1 is a zombie/defunct process, per /proc/<pid>/stat's
# state field. comm (2nd field) can itself contain spaces/parens, so the
# state is found by stripping the longest "...) " prefix, not by naive
# field-splitting.
is_zombie() {
  local pid="$1" stat rest state
  stat="$(cat "/proc/${pid}/stat" 2>/dev/null)" || return 1
  rest="${stat##*) }"
  state="${rest%% *}"
  [[ "$state" == "Z" ]]
}

# Prints "pid state ppid" for every live (non-zombie) pid in /proc.
list_live_processes() {
  local dir pid stat rest state ppid_rest ppid
  for dir in /proc/[0-9]*; do
    pid="${dir#/proc/}"
    [[ -r "${dir}/stat" ]] || continue
    stat="$(cat "${dir}/stat" 2>/dev/null)" || continue
    rest="${stat##*) }"
    state="${rest%% *}"
    [[ "$state" == "Z" ]] && continue
    ppid_rest="${rest#* }"
    ppid="${ppid_rest%% *}"
    printf '%s %s\n' "$pid" "$ppid"
  done
}

# Resolves the real Linh root pid by matching /proc/<pid>/exe against
# $1 (resolved with readlink -f), ignoring zombies and anything that
# doesn't match. Errors if zero or more than one candidate is found.
discover_root_pid() {
  local exe_target="$1" dir pid link
  local -a candidates=()
  for dir in /proc/[0-9]*; do
    pid="${dir#/proc/}"
    [[ -e "${dir}/exe" ]] || continue
    is_zombie "$pid" && continue
    link="$(readlink -f "${dir}/exe" 2>/dev/null)" || continue
    if [[ "$link" == "$exe_target" ]]; then
      candidates+=("$pid")
    fi
  done
  case "${#candidates[@]}" in
    0) err "no running process found with exe matching '${exe_target}' (pass --pid or --exe)" ;;
    1) printf '%s\n' "${candidates[0]}" ;;
    *) err "multiple processes match exe '${exe_target}' (${candidates[*]}); pass --pid to disambiguate" ;;
  esac
}

# Prints $1 (the root pid) and every live descendant pid, in BFS order,
# one per line.
collect_tree_pids() {
  local root="$1" pid ppid line
  local -A children=()
  while read -r pid ppid; do
    children["$ppid"]="${children[$ppid]:-}${children[$ppid]:+ }${pid}"
  done < <(list_live_processes)

  local -a queue=("$root") result=()
  while [[ ${#queue[@]} -gt 0 ]]; do
    local current="${queue[0]}"
    queue=("${queue[@]:1}")
    result+=("$current")
    local -a kids=()
    if [[ -n "${children[$current]:-}" ]]; then
      read -ra kids <<<"${children[$current]}"
    fi
    for pid in "${kids[@]}"; do
      queue+=("$pid")
    done
  done
  printf '%s\n' "${result[@]}"
}

# --- smaps_rollup parsing ---------------------------------------------

# Prints the kB value of $2 (e.g. "Pss") from smaps_rollup file $1.
read_rollup_kb() {
  local file="$1" key="$2" line
  line="$(grep -m1 "^${key}:" "$file" 2>/dev/null)" || return 1
  awk '{print $2}' <<<"$line"
}

# Prints "rss pss pss_anon pss_file uss swap_pss" (all kB) for pid $1,
# or fails if its smaps_rollup can't be read.
process_metrics() {
  local pid="$1"
  local file="/proc/${pid}/smaps_rollup"
  local rss pss pss_anon pss_file priv_clean priv_dirty swap_pss
  [[ -r "$file" ]] || return 1
  rss="$(read_rollup_kb "$file" "Rss")" || return 1
  pss="$(read_rollup_kb "$file" "Pss")" || return 1
  pss_anon="$(read_rollup_kb "$file" "Pss_Anon")" || pss_anon=0
  pss_file="$(read_rollup_kb "$file" "Pss_File")" || pss_file=0
  priv_clean="$(read_rollup_kb "$file" "Private_Clean")" || priv_clean=0
  priv_dirty="$(read_rollup_kb "$file" "Private_Dirty")" || priv_dirty=0
  swap_pss="$(read_rollup_kb "$file" "SwapPss")" || swap_pss=0
  printf '%s %s %s %s %s %s\n' "$rss" "$pss" "$pss_anon" "$pss_file" \
    "$((priv_clean + priv_dirty))" "$swap_pss"
}

# --- formatting ----------------------------------------------------

fmt_kb_mib() {
  local kb="$1"
  awk -v k="$kb" 'BEGIN{printf "%d KiB (%.1f MiB)", k, k/1024}'
}

# --- snapshot ----------------------------------------------------

# Globals set by print_snapshot for the caller (guided mode) to read.
LAST_PID_COUNT=0
LAST_TOTAL_PSS=0
LAST_TOTAL_PSS_ANON=0
LAST_TOTAL_PSS_FILE=0
LAST_TOTAL_USS=0
LAST_TOTAL_SWAP=0

# Prints one labeled table for the tree rooted at $1, over pids $2..$n.
print_snapshot() {
  local label="$1"
  shift
  local -a pids=("$@")
  local pid comm metrics rss pss pss_anon pss_file uss swap_pss
  local total_pss=0 total_pss_anon=0 total_pss_file=0 total_uss=0 total_swap=0
  local counted=0

  printf '=== %s ===\n' "$label"
  printf '%-8s %-20s %10s %10s %10s %10s %10s %10s\n' \
    "PID" "NAME" "RSS" "PSS" "PSS_ANON" "PSS_FILE" "USS" "SWAP_PSS"

  for pid in "${pids[@]}"; do
    if ! metrics="$(process_metrics "$pid")"; then
      continue
    fi
    read -r rss pss pss_anon pss_file uss swap_pss <<<"$metrics"
    comm="$(tr -d '\n' <"/proc/${pid}/comm" 2>/dev/null)" || comm="?"
    printf '%-8s %-20s %10s %10s %10s %10s %10s %10s\n' \
      "$pid" "$comm" "$rss" "$pss" "$pss_anon" "$pss_file" "$uss" "$swap_pss"
    total_pss=$((total_pss + pss))
    total_pss_anon=$((total_pss_anon + pss_anon))
    total_pss_file=$((total_pss_file + pss_file))
    total_uss=$((total_uss + uss))
    total_swap=$((total_swap + swap_pss))
    counted=$((counted + 1))
  done

  if [[ "$counted" -eq 0 ]]; then
    err "no process in the tree had a readable smaps_rollup"
  fi

  printf -- '--- TREE TOTAL (n=%d processes; PSS is the headline, RSS above is per-process only and is never summed) ---\n' "$counted"
  printf '  PSS       : %s\n' "$(fmt_kb_mib "$total_pss")"
  printf '  PSS_ANON  : %s\n' "$(fmt_kb_mib "$total_pss_anon")"
  printf '  PSS_FILE  : %s\n' "$(fmt_kb_mib "$total_pss_file")"
  printf '  USS       : %s\n' "$(fmt_kb_mib "$total_uss")"
  printf '  SWAP_PSS  : %s\n' "$(fmt_kb_mib "$total_swap")"

  LAST_PID_COUNT="$counted"
  LAST_TOTAL_PSS="$total_pss"
  LAST_TOTAL_PSS_ANON="$total_pss_anon"
  LAST_TOTAL_PSS_FILE="$total_pss_file"
  LAST_TOTAL_USS="$total_uss"
  LAST_TOTAL_SWAP="$total_swap"
}

append_tsv_row() {
  local out_file="$1" label="$2"
  if [[ ! -s "$out_file" ]]; then
    printf 'timestamp\tlabel\tpid_count\tpss_kib\tpss_anon_kib\tpss_file_kib\tuss_kib\tswap_pss_kib\n' >"$out_file"
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$(date +%s)" "$label" "$LAST_PID_COUNT" "$LAST_TOTAL_PSS" \
    "$LAST_TOTAL_PSS_ANON" "$LAST_TOTAL_PSS_FILE" "$LAST_TOTAL_USS" "$LAST_TOTAL_SWAP" >>"$out_file"
}

# --- modes ----------------------------------------------------

resolve_root_pid() {
  local pid_opt="$1" exe_opt="$2"
  local root_pid
  if [[ -n "$pid_opt" ]]; then
    root_pid="$pid_opt"
  else
    local exe_target
    exe_target="$(readlink -f "$exe_opt" 2>/dev/null)" || exe_target="$exe_opt"
    root_pid="$(discover_root_pid "$exe_target")"
  fi
  [[ -d "/proc/${root_pid}" ]] || err "no such process: pid ${root_pid}"
  [[ -r "/proc/${root_pid}/smaps_rollup" ]] || err "cannot read /proc/${root_pid}/smaps_rollup (permission denied, or pid ${root_pid} is not Linh)"
  printf '%s\n' "$root_pid"
}

cmd_snapshot() {
  local label="" pid_opt="" exe_opt="$DEFAULT_EXE" out_file=""
  [[ $# -ge 1 ]] || err "snapshot requires a <label> argument"
  label="$1"
  shift
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --pid)
        pid_opt="${2:?--pid requires a value}"
        shift 2
        ;;
      --exe)
        exe_opt="${2:?--exe requires a value}"
        shift 2
        ;;
      --out)
        out_file="${2:?--out requires a value}"
        shift 2
        ;;
      *) err "unknown argument: $1" ;;
    esac
  done

  local root_pid
  root_pid="$(resolve_root_pid "$pid_opt" "$exe_opt")"
  local -a pids=()
  mapfile -t pids < <(collect_tree_pids "$root_pid")
  print_snapshot "$label" "${pids[@]}"
  if [[ -n "$out_file" ]]; then
    append_tsv_row "$out_file" "$label"
  fi
}

cmd_guided() {
  local pid_opt="" exe_opt="$DEFAULT_EXE" out_file="" settle="$DEFAULT_SETTLE"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --pid)
        pid_opt="${2:?--pid requires a value}"
        shift 2
        ;;
      --exe)
        exe_opt="${2:?--exe requires a value}"
        shift 2
        ;;
      --out)
        out_file="${2:?--out requires a value}"
        shift 2
        ;;
      --settle)
        settle="${2:?--settle requires a value}"
        shift 2
        ;;
      *) err "unknown argument: $1" ;;
    esac
  done

  local root_pid
  root_pid="$(resolve_root_pid "$pid_opt" "$exe_opt")"

  local -a labels=(idle settings prompt closed)
  local -a prompts=(
    "Make sure Linh is running and idle in the tray (no Settings or prompt window open)."
    "Open Linh's Settings window now (tray icon -> Settings)."
    "Trigger the prompt window now (open a link that routes to the prompt picker)."
    "Close every Linh window again (back to idle in the tray)."
  )
  local -A pss_by_label=()

  local i label
  for i in "${!labels[@]}"; do
    label="${labels[$i]}"
    echo
    echo ">>> Step $((i + 1))/4: ${prompts[$i]}"
    read -r -p "Press Enter when ready... " _
    echo "Waiting ${settle}s to settle..."
    sleep "$settle"
    local -a pids=()
    mapfile -t pids < <(collect_tree_pids "$root_pid")
    print_snapshot "$label" "${pids[@]}"
    pss_by_label["$label"]="$LAST_TOTAL_PSS"
    if [[ -n "$out_file" ]]; then
      append_tsv_row "$out_file" "$label"
    fi
  done

  echo
  echo "=== SUMMARY (PSS vs idle) ==="
  printf '%-10s %20s %20s\n' "STATE" "PSS" "DELTA_VS_IDLE"
  local idle_pss="${pss_by_label[idle]}"
  for label in "${labels[@]}"; do
    local delta=$((pss_by_label[$label] - idle_pss))
    printf '%-10s %20s %20s\n' "$label" "$(fmt_kb_mib "${pss_by_label[$label]}")" "$(fmt_kb_mib "$delta")"
  done
}

main() {
  [[ $# -ge 1 ]] || {
    usage
    exit 1
  }
  local mode="$1"
  shift
  case "$mode" in
    snapshot) cmd_snapshot "$@" ;;
    guided) cmd_guided "$@" ;;
    -h | --help) usage ;;
    *)
      usage
      err "unknown mode: ${mode}"
      ;;
  esac
}

main "$@"
