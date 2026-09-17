#!/usr/bin/env bash
# Every check that needs the media corpus, in one run with one verdict.
#
# CI cannot do this: the corpus is licensed material, tens of gigabytes of it,
# and no public runner may hold it. So the checks that read it live here, and
# the owner runs them before a release. Each step prints its exit code; the
# script exits non-zero if any step failed, and names them at the end.
#
#   tools/media_regression.sh [media-directory] [log-directory]
#
# The media directory defaults to $OADEC_MEDIA, the log directory to
# media-regression/ beside it. The ADM stages are run when the Python
# environment of docs/audit/adm is there, and skipped, not failed, when it is
# not: they need Dolby's tools.
set -u
MEDIA=${1:-${OADEC_MEDIA:-}}
if [ -z "$MEDIA" ] || [ ! -d "$MEDIA" ]; then
  echo "usage: tools/media_regression.sh <media-directory> [log-directory]" >&2
  echo "       (or set OADEC_MEDIA)" >&2
  exit 2
fi
LOGS=${2:-$MEDIA/media-regression}
mkdir -p "$LOGS"
FAILED=()
step() {
  local name=$1
  shift
  local started
  started=$(date +%s)
  "$@" > "$LOGS/$name.log" 2>&1
  local rc=$?
  printf '%-22s rc=%-3s %4ss  %s\n' "$name" "$rc" "$(( $(date +%s) - started ))" "$LOGS/$name.log"
  [ "$rc" -eq 0 ] || FAILED+=("$name")
  return 0
}

echo "media: $MEDIA"
echo "logs:  $LOGS"
cargo build --release --locked -p oadec-cli > "$LOGS/build.log" 2>&1 || {
  echo "the release build failed; see $LOGS/build.log" >&2
  exit 1
}
BIN=$(cd "$(dirname "$0")/.." && pwd)/target/release/oadec
[ -x "$BIN" ] || BIN="$BIN.exe"
export OADEC_BIN="$BIN"

# The media tests, and the same tests without the corpus: a suite that passes
# because it found nothing to read is the failure this guards against.
step media-suite env OADEC_MEDIA="$MEDIA" cargo test --release --locked -p oadec-cli --test real -- --ignored
if env -u OADEC_MEDIA cargo test --release --locked -p oadec-cli --test real -- --ignored \
    > "$LOGS/media-negative.log" 2>&1; then
  echo "media-negative            the suite passed without the corpus, which it must not"
  FAILED+=("media-negative")
else
  echo "media-negative         rc=ok        $LOGS/media-negative.log"
fi

# The project's own gates.
step truehd-gates /usr/bin/bash tools/regression_gates.sh "$MEDIA" "$LOGS/gates.json"
step joc-object-gate python tools/joc_object_gate.py --work "$MEDIA" --binary "$BIN" --out "$LOGS/objgate.json"

# The corruption replays: the same seeds, the same counts.
step replay-joc python tools/replay_fuzz.py "$MEDIA/audit/tmp/fz-joc.ec3" joc 1 120 "$LOGS/replay-joc.json"
step replay-truehd python tools/replay_fuzz.py "$MEDIA/audit/tmp/fz-thd.thd" truehd 7 100 "$LOGS/replay-truehd.json"

# The ADM stages need Dolby's tools and the Python environment of the audit.
ADM_PY="$MEDIA/audit/adm/venv/Scripts/python.exe"
if [ -x "$ADM_PY" ]; then
  step adm-harness "$ADM_PY" docs/audit/adm-remediation/tools/run_remediation.py \
    --repo . --work "$LOGS/adm" --media "$MEDIA" --stage harness
else
  echo "adm-harness            skipped      (no $ADM_PY)"
fi

if [ ${#FAILED[@]} -eq 0 ]; then
  echo "all media checks passed"
  exit 0
fi
echo "failed: ${FAILED[*]}"
exit 1
