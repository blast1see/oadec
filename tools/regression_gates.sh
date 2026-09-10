#!/usr/bin/env bash
# The results the audit proved, re-run.
#
# TrueHD is lossless, so the only acceptable answer is zero differing samples,
# against each of the three decoders that can be asked. Anything else means a
# change somewhere else in the tree broke the one thing about this decoder that
# is provable rather than bounded.
#
#   bash tools/regression_gates.sh <work-dir> <out.json>
#
# The work directory is the one `OADEC_MEDIA` points at. References are
# regenerated rather than stored: they are gigabytes and the tools are on hand.
set -u
WORK="${1:?usage: regression_gates.sh <work-dir> <out.json>}"
OUT="${2:?usage: regression_gates.sh <work-dir> <out.json>}"
BIN="${OADEC_BIN:-$HOME/Documents/oadec/target/release/oadec.exe}"
TRUEHDD="${TRUEHDD:-$HOME/.cargo/bin/truehdd}"
TMP="$WORK/audit/remediation/tmp/gates"
mkdir -p "$TMP"
THD="$WORK/clips/pi-head50m.thd"

json_escape() { python -c "import json,sys;print(json.dumps(sys.stdin.read()))"; }
results=()

record() { # name verdict detail
  results+=("{\"gate\":\"$1\",\"verdict\":\"$2\",\"detail\":$(printf '%s' "$3" | json_escape)}")
}

echo "== D1: presentation 2 against FFmpeg =="
if [ -f "$THD" ]; then
  ffmpeg -v error -y -i "$THD" -f s32le -acodec pcm_s32le "$TMP/pi-p2-ffmpeg.s32" 2>"$TMP/ffmpeg.log"
  out=$("$BIN" compare -p 2 -r "$TMP/pi-p2-ffmpeg.s32" --reference-format s32le "$THD" 2>&1)
  rc=$?
  echo "$out" | tail -5
  if echo "$out" | grep -q "result: BIT-EXACT" && [ $rc -eq 0 ]; then
    record "truehd-presentation-2-vs-ffmpeg" "PASS" "$out"
  else
    record "truehd-presentation-2-vs-ffmpeg" "FAIL" "$out"
  fi
  rm -f "$TMP/pi-p2-ffmpeg.s32"
else
  record "truehd-presentation-2-vs-ffmpeg" "NOT RUN" "$THD is missing"
fi

echo "== D2: presentation 3 against truehdd =="
if [ -f "$THD" ] && [ -x "$TRUEHDD" ]; then
  rm -rf "$TMP/thdd"; mkdir -p "$TMP/thdd"
  "$TRUEHDD" decode --loglevel error --presentation 3 --output-path "$TMP/thdd/pi" "$THD" \
    > "$TMP/truehdd.log" 2>&1
  ref=$(ls "$TMP/thdd"/*.atmos.audio 2>/dev/null | head -1)
  ours="$TMP/pi-p3-ours"
  "$BIN" decode -p 3 --format damf -o "$ours" "$THD" > "$TMP/ours-p3.log" 2>&1
  if [ -n "$ref" ] && [ -f "$ours.atmos.audio" ]; then
    out=$(python - "$ref" "$ours.atmos.audio" <<'PY'
import sys, struct
# CAF: compare the payload of the `data` chunk of each file, byte for byte
def payload(path):
    with open(path, 'rb') as f:
        d = f.read()
    i = 8
    while i + 12 <= len(d):
        name = d[i:i+4]; size = struct.unpack('>q', d[i+4:i+12])[0]
        body = i + 12
        if name == b'data':
            end = len(d) if size <= 0 else body + size
            return d[body+4:end]          # the data chunk opens with mEditCount
        if size <= 0:
            break
        i = body + size
    return b''
a, b = payload(sys.argv[1]), payload(sys.argv[2])
n = min(len(a), len(b))
diff = sum(1 for i in range(0, n, 4096) if a[i:i+4096] != b[i:i+4096])
print(f"reference {len(a)} bytes, ours {len(b)} bytes, common {n}")
print(f"differing 4 KiB blocks: {diff}")
print("result: BIT-EXACT" if a == b else "result: DIFFERENT")
PY
)
    echo "$out"
    if echo "$out" | grep -q "result: BIT-EXACT"; then
      record "truehd-presentation-3-vs-truehdd" "PASS" "$out"
    else
      record "truehd-presentation-3-vs-truehdd" "FAIL" "$out"
    fi
  else
    record "truehd-presentation-3-vs-truehdd" "NOT RUN" "no reference produced"
  fi
  rm -rf "$TMP/thdd" "$ours".atmos*
else
  record "truehd-presentation-3-vs-truehdd" "NOT RUN" "truehdd or the stream is missing"
fi

echo "== D3: presentation 3 against Dolby =="
DOLBY="$WORK/ref-oar/pi-thd-obj.s32"
if [ -f "$THD" ] && [ -f "$DOLBY" ]; then
  ours="$TMP/pi-p3-dolby.pcm"
  "$BIN" decode -p 3 --format pcm --order stream -o "$ours" "$THD" > "$TMP/ours-p3b.log" 2>&1
  out=$(python - "$DOLBY" "$ours" <<'PY'
import sys, numpy as np
# Dolby writes 16 object channels of 24-bit samples in 32-bit words; oadec
# writes the objects the stream actually carries, and Dolby pads the rest with
# digital silence, so the comparison is over the channels oadec produced.
ref = np.fromfile(sys.argv[1], dtype='<i4')
ours = np.fromfile(sys.argv[2], dtype='<u1')
n = len(ours) // 3
ours = ours[:n*3].reshape(n, 3).astype(np.int32)
ours = (ours[:,0] | (ours[:,1] << 8) | (ours[:,2] << 16))
ours = np.where(ours & 0x800000, ours - (1 << 24), ours)
ref16 = ref.reshape(-1, 16)
# Dolby's samples are 24-bit left-justified in 32-bit words
ref16 = ref16 >> 8
for nch in (12, 16):
    if len(ours) % nch:
        continue
    ours_n = ours.reshape(-1, nch)
    m = min(len(ours_n), len(ref16))
    if m == 0:
        continue
    d = ours_n[:m] - ref16[:m, :nch]
    print(f"{nch} channels: {m} frames, {int((d != 0).sum())} differing samples, "
          f"max |diff| {int(np.abs(d).max())}")
    if (d == 0).all():
        print("result: BIT-EXACT")
        break
else:
    print("result: DIFFERENT")
PY
)
  echo "$out"
  if echo "$out" | grep -q "result: BIT-EXACT"; then
    record "truehd-presentation-3-vs-dolby" "PASS" "$out"
  else
    record "truehd-presentation-3-vs-dolby" "FAIL" "$out"
  fi
  rm -f "$ours"
else
  record "truehd-presentation-3-vs-dolby" "NOT RUN" "the stream or Dolby's object dump is missing"
fi

printf '{"gates":[%s]}\n' "$(IFS=,; echo "${results[*]}")" > "$OUT"
python -c "
import json,sys
d=json.load(open(sys.argv[1]))
for g in d['gates']: print(f\"{g['verdict']:8s} {g['gate']}\")
" "$OUT"
