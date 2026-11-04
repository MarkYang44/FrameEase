#!/usr/bin/env bash
# Run one CI command; if it fails, repeat what went wrong as GitHub Actions error
# annotations. Annotations show on the run's summary page and in the public API, so a
# failure can be read without signing in to open the full log.
#
#   bash scripts/ci-step.sh "<title>" <command> [args...]
#
# For `cargo test` output, each failing test's captured output becomes its own
# annotation; in every case the last lines of output are added as well.
set -uo pipefail

title="$1"
shift

work="$(mktemp -d)"
log="$work/output.log"
"$@" 2>&1 | tee "$log"
status=${PIPESTATUS[0]}

# Workflow-command escaping (property values also escape ':' and ',').
esc_data() {
  local s="$1"
  s="${s//'%'/%25}"
  s="${s//$'\r'/%0D}"
  s="${s//$'\n'/%0A}"
  printf '%s' "$s"
}
esc_prop() {
  local s
  s="$(esc_data "$1")"
  s="${s//':'/%3A}"
  s="${s//','/%2C}"
  printf '%s' "$s"
}
annotate() { # title, file
  echo "::error title=$(esc_prop "$1")::$(esc_data "$(tail -c 8000 "$2")")"
}

if [ "$status" -ne 0 ]; then
  # libtest prints a "---- <test> stdout ----" block per failing test before its
  # "failures:" summary list. GitHub shows at most 10 error annotations per step.
  awk -v dir="$work" '
    /^---- .* stdout ----$/ { if (f) close(f); n++; f = sprintf("%s/block%03d", dir, n) }
    /^failures:$/ { if (f) close(f); f = "" }
    f { print > f }
  ' "$log"
  shown=0
  for block in "$work"/block*; do
    [ -e "$block" ] && [ "$shown" -lt 8 ] || break
    name="$(head -n 1 "$block" | sed -e 's/^---- //' -e 's/ stdout ----$//')"
    annotate "$title - $name" "$block"
    shown=$((shown + 1))
  done
  tail -n 60 "$log" > "$work/tail"
  annotate "$title failed (exit $status)" "$work/tail"
fi

rm -rf "$work"
exit "$status"
