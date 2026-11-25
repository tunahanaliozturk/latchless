#!/usr/bin/env bash
# Runs loom tests that must FAIL, for a known-wrong variant built with
# `--cfg latchless_mutation=...` in RUSTFLAGS (docs/adr/0003-known-wrong-variants.md).
#
# Two steps, so that a build failure cannot pass for a rejection:
#   1. build the test binary with `cargo test --no-run`; if that fails, so does this;
#   2. run the tests and succeed only if the run fails, is not a timeout, and its
#      output shows that a test started and then failed (a loom finding, a panic,
#      or a failed test). A pass means the loom tests have gone blind; a timeout
#      is a hang, not a rejection. The exit code alone is not enough: when loom
#      panics during unwinding the test binary aborts, which shows up as 101 or
#      as a signal.
#
# Usage: scripts/expect-loom-failure.sh <test filter>
set -u

filter="${1:?usage: expect-loom-failure.sh <test filter>}"
log="$(mktemp)"

if ! cargo test --lib --release --no-run; then
  echo "expect-loom-failure: the variant does not build; that is not loom rejecting it"
  exit 1
fi

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
if grep -q -E "^running [1-9][0-9]* tests?" "$log" &&
  grep -q -e "Causality violation" -e "panicked at" -e "test result: FAILED" -e "\.\.\. FAILED" "$log"; then
  echo "expect-loom-failure: rejected, as it should be (exit $code)"
  exit 0
fi
echo "expect-loom-failure: the run failed (exit $code) but no test is shown running and failing; see above"
exit 1
