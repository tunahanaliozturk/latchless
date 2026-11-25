#!/usr/bin/env bash
# Runs loom tests that must FAIL, for a known-wrong variant built with
# `--cfg latchless_mutation=...` in RUSTFLAGS (docs/adr/0003-known-wrong-variants.md).
#
# Succeeds only if the run fails with a loom finding or a failed assertion.
# A pass means the loom tests have gone blind; a timeout is a hang, not a
# rejection. The exit code alone is not enough: when loom panics during
# unwinding the test binary aborts, which shows up as 101 or as a signal.
#
# Usage: scripts/expect-loom-failure.sh <test filter>
set -u

filter="${1:?usage: expect-loom-failure.sh <test filter>}"
log="$(mktemp)"

timeout 600 cargo test --lib --release "$filter" >"$log" 2>&1
code=$?
cat "$log"

if [ "$code" -eq 124 ]; then
  echo "expect-loom-failure: timed out; a hang is not a rejection"
  exit 1
fi
if [ "$code" -eq 0 ]; then
  echo "expect-loom-failure: the known-wrong variant passed; the loom tests did not notice"
  exit 1
fi
if grep -q -e "Causality violation" -e "assertion" -e "test result: FAILED" "$log"; then
  echo "expect-loom-failure: rejected, as it should be (exit $code)"
  exit 0
fi
echo "expect-loom-failure: the run failed (exit $code) but not with a test failure; see above"
exit 1
