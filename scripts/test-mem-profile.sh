#!/usr/bin/env bash
# A small, dependency-free smoke test for scripts/mem-profile.sh: runs
# `snapshot` against the current shell's own pid (via --pid $$, always
# running and always readable) and asserts the output format, the
# --out TSV file, and that invalid input is rejected. Run directly:
#   scripts/test-mem-profile.sh
set -euo pipefail
shopt -s inherit_errexit

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MEM_PROFILE="${SCRIPT_DIR}/mem-profile.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

failures=0

check() {
  local description="$1" condition="$2"
  if [[ "$condition" == "0" ]]; then
    echo "ok - ${description}"
  else
    echo "NOT OK - ${description}"
    failures=$((failures + 1))
  fi
}

contains() {
  local haystack="$1" needle="$2"
  [[ "$haystack" == *"$needle"* ]]
}

# --- snapshot against the current shell's own pid ---------------------

if output="$("$MEM_PROFILE" snapshot self-test --pid "$$" 2>&1)"; then
  status=0
else
  status=$?
fi
check "snapshot against \$\$ exits 0" "$status"

contains "$output" "=== self-test ===" && label_ok=0 || label_ok=1
check "prints the requested label as a section header" "$label_ok"

contains "$output" "PID" && contains "$output" "RSS" && contains "$output" "PSS" \
  && header_ok=0 || header_ok=1
check "prints a column header with PID/RSS/PSS" "$header_ok"

contains "$output" "TREE TOTAL" && total_ok=0 || total_ok=1
check "prints a TREE TOTAL section" "$total_ok"

contains "$output" "PSS       :" && pss_total_ok=0 || pss_total_ok=1
check "TREE TOTAL reports a PSS line" "$pss_total_ok"

contains "$output" "KiB" && contains "$output" "MiB" && units_ok=0 || units_ok=1
check "reports both KiB and MiB" "$units_ok"

# RSS must never appear as a *summed* total line (only per-process rows
# and the explicit "RSS above is per-process only" note may mention it).
if contains "$output" "RSS       :"; then
  rss_not_summed=1
else
  rss_not_summed=0
fi
check "RSS is never printed as a TREE TOTAL line" "$rss_not_summed"

contains "$output" "$$" && own_pid_ok=0 || own_pid_ok=1
check "the current shell's own pid appears in the tree" "$own_pid_ok"

# --- --out TSV ---------------------------------------------------------

out_file="${TMP_DIR}/mem.tsv"
"$MEM_PROFILE" snapshot tsv-test --pid "$$" --out "$out_file" >/dev/null
if [[ -s "$out_file" ]]; then
  tsv_exists_ok=0
else
  tsv_exists_ok=1
fi
check "--out creates a non-empty TSV file" "$tsv_exists_ok"

header_line="$(head -n1 "$out_file")"
contains "$header_line" "pss_kib" && tsv_header_ok=0 || tsv_header_ok=1
check "TSV file has a header row with pss_kib" "$tsv_header_ok"

row_count="$(($(wc -l <"$out_file") - 1))"
if [[ "$row_count" -eq 1 ]]; then
  tsv_row_ok=0
else
  tsv_row_ok=1
fi
check "TSV file has exactly one data row after one snapshot" "$tsv_row_ok"

# A second snapshot must append, not overwrite, and not repeat the header.
"$MEM_PROFILE" snapshot tsv-test-2 --pid "$$" --out "$out_file" >/dev/null
header_count="$(grep -c "pss_kib" "$out_file")"
if [[ "$header_count" -eq 1 ]]; then
  tsv_single_header_ok=0
else
  tsv_single_header_ok=1
fi
check "a second snapshot appends without repeating the TSV header" "$tsv_single_header_ok"

# --- --out refuses to follow a symlink ----------------------------------

symlink_target="${TMP_DIR}/real-target.tsv"
symlink_path="${TMP_DIR}/via-symlink.tsv"
: >"$symlink_target"
ln -s "$symlink_target" "$symlink_path"

if "$MEM_PROFILE" snapshot bad --pid "$$" --out "$symlink_path" >/dev/null 2>"${TMP_DIR}/err_symlink"; then
  symlink_rejected=1
else
  symlink_rejected=0
fi
check "--out refuses to write through an existing symlink" "$symlink_rejected"

[[ -s "${TMP_DIR}/err_symlink" ]] && symlink_msg_ok=0 || symlink_msg_ok=1
check "--out symlink refusal prints a clear error message" "$symlink_msg_ok"

if [[ -s "$symlink_target" ]]; then
  symlink_target_untouched=1
else
  symlink_target_untouched=0
fi
check "the symlink's target file is left untouched (empty, not written through)" "$symlink_target_untouched"

# --- error paths ---------------------------------------------------------

if "$MEM_PROFILE" snapshot bad --pid 999999999 >/dev/null 2>"${TMP_DIR}/err1"; then
  bad_pid_ok=1
else
  bad_pid_ok=0
fi
check "an invalid --pid exits non-zero" "$bad_pid_ok"
[[ -s "${TMP_DIR}/err1" ]] && bad_pid_msg_ok=0 || bad_pid_msg_ok=1
check "an invalid --pid prints a clear error message" "$bad_pid_msg_ok"

if "$MEM_PROFILE" snapshot bad --exe /usr/bin/this-binary-does-not-exist >/dev/null 2>"${TMP_DIR}/err2"; then
  bad_exe_ok=1
else
  bad_exe_ok=0
fi
check "an --exe matching no process exits non-zero" "$bad_exe_ok"

if "$MEM_PROFILE" >/dev/null 2>"${TMP_DIR}/err3"; then
  no_args_ok=1
else
  no_args_ok=0
fi
check "running with no arguments exits non-zero (prints usage)" "$no_args_ok"

echo
if [[ "$failures" -eq 0 ]]; then
  echo "All checks passed."
  exit 0
else
  echo "${failures} check(s) failed."
  exit 1
fi
