# ADR 0005: Diagnose intermittent concurrent-adapter failures

## Decision

A Windows CI run on the evidence-only PR exposed an intermittent concurrent fixture failure: one adapter ended before producing JSON, so the gate correctly returned incomplete. The previous protocol error omitted subprocess stderr and exit status, preventing the captured log from identifying the exact originating exception. Preserve those diagnostics with a 2,000-character bound and exercise twelve independent concurrency batches per acceptance run. Do not retry a failed scan internally or translate empty output to success.

## Compatibility

Only incomplete-response diagnostics become more informative. The adapter protocol, required/error semantics, two-worker test assertion, subprocess ownership and output limits remain unchanged. This decision does not presume the failure was a product or fixture bug before collecting its stderr; record the observed cause and subsequent repair in delivery evidence.

## Performance

Successful scan execution is unchanged. Diagnostic formatting runs only after response validation fails. Repeated concurrency probes add test coverage, not production retries or new runtime processes. Existing native and semantic performance budgets remain unchanged.

## Verification

A malformed-response fixture proves its bounded stderr is included in the gate error. Repeated concurrency tests preserve the exact maximum of two active adapters and deterministic coverage order. Native Windows CI must expose any underlying exception and must pass after its cause is repaired; no check is disabled or waived. All regular architecture, compiler, package and performance checks remain required.

## Review authorization

The owner explicitly delegated review, repair and merge to the assistant. This is a continuation of that authorization and uses normal protected PRs. The initial failing evidence PR remains unmerged while the intermittent failure is investigated. No independent human or administrator bypass is required or represented.

## Follow-up verification and fixture repair

Run 35100451203 passed all platform checks after bounded stderr/exit diagnostics and twelve concurrency batches were added. The original originating Windows exception was not retained by the old diagnostic path, so that run cannot prove its precise cause retrospectively. Do not label it a proven product race or silently erase the failed run.

The reviewed fixture used directory removal/recreation as a cross-process lock. Replace that platform-sensitive observation helper with bounded SQLite transactions from Python's standard library. The production scheduler and process runner are unchanged. Preserve the exact maximum-two-active assertion across twelve batches, and additionally assert zero active rows after every batch. The transaction timeout remains bounded and any lock/fixture exception still fails the gate; no scan retries or ignored failures are introduced. Native CI must pass this stronger fixture before merge.
