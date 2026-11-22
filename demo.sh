#!/bin/bash
# Echolocation demonstration — three acts, separate processes, one shared
# .phext medium, no prior introduction between any of the programs.
#
#   Act 1: mira probes universe 1.1.1; ni-knight-runner answers.
#   Act 2: mira probes across into universe 2.1.1; far-seat answers there,
#          and the echo crosses back into 1.1.1.
#   Act 3: mira probes universe 3.1.1, where nobody listens. Silence.
#
# Run from the repo root after `cargo build`. Output is the transcript.
set -u
cd "$(dirname "$0")"
BIN=./target/debug/echolocate
DEMO=demo-run
rm -rf "$DEMO"
mkdir -p "$DEMO"
MEDIUM="$DEMO/medium.phext"

echo "=== Act 1: discovery in universe 1.1.1 ==="
"$BIN" answer --medium "$MEDIUM" --name ni-knight-runner \
  --home 1.1.1/1.1.1/9.4.1 --capabilities time-census,consensus,blend \
  --state "round 34 complete; consensus +0.006 ms" \
  --mailbox 1.1.1/1.4.1/1.1.1 --poll-ms 100 --for 8 &
sleep 1
"$BIN" probe --medium "$MEDIUM" --name mira \
  --home 1.1.1/1.1.1/9.1.1 --capabilities time-census \
  --intent time-census --ttl 4 --mode all --poll-ms 100
wait
echo
echo "=== Act 2: a probe across universes (1.1.1 -> 2.1.1) ==="
"$BIN" answer --medium "$MEDIUM" --name far-seat \
  --home 2.1.1/1.1.1/9.4.1 --capabilities cartography \
  --universe 2.1.1 --state "surveying the far shelf" \
  --mailbox 2.1.1/1.4.1/1.1.1 --poll-ms 100 --for 8 &
sleep 1
"$BIN" probe --medium "$MEDIUM" --name mira \
  --home 1.1.1/1.1.1/9.1.1 --target 2.1.1 \
  --intent cartography --ttl 6 --mode first --poll-ms 100
wait
echo
echo "=== Act 3: a probe into silence (universe 3.1.1) ==="
"$BIN" probe --medium "$MEDIUM" --name mira \
  --home 1.1.1/1.1.1/9.1.1 --target 3.1.1 \
  --intent any --ttl 3 --mode all --poll-ms 100
echo
echo "=== The medium afterwards: $MEDIUM ($(wc -c < "$MEDIUM") bytes) ==="
