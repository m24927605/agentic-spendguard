# D07 — Microsoft Agent Framework (MAF) middleware — tests.md

> Status: Proposed. Sibling: `design.md`, `implementation.md`, `acceptance.md`, `review-standards.md`.
> Coverage matrix scope: per-slice unit + integration + demo regression.

## 1. Test pyramid

| Layer | .NET location | Python location | What it covers |
|-------|---------------|-----------------|----------------|
| Unit | `sdk/dotnet/tests/Spendguard.AgentFramework.Tests/Unit/` | `sdk/python/tests/integrations/agent_framework/test_unit.py` | Token estimator, idempotency-key derivation, options binding, exception types |
| Integration | `sdk/dotnet/tests/Spendguard.AgentFramework.Tests/Integration/` | `sdk/python/tests/integrations/agent_framework/test_integration.py` | Middleware ↔ sidecar stub round-trip, allow / deny / degrade outcomes, replay-safety |
| Demo regression | `deploy/demo/tests/test_maf_dotnet.py` + `test_maf_python.py` | same | Full MAF + sidecar + Postgres stack against recorded provider fixture |

## 2. Unit tests (slices 4 + 7)

### 2.1 .NET (xUnit + Moq + Verify)

| Test | Asserts |
|------|---------|
| `TokenEstimator_OpenAiModels_MatchesSharpToken` | `SharpTokenEstimator` output equals reference `SharpToken` count for gpt-4o-mini, gpt-4o, gpt-3.5-turbo on a 4-message fixture |
| `TokenEstimator_UnknownModel_FallsBackToSidecar` | Calls `SidecarTokenEstimator.CountTokensAsync` mock |
| `IdempotencyKey_Stable_AcrossInvocations` | Same `(tenantId, sessionId, runId, stepId, llmCallId, trigger)` → identical blake2b key bytes |
| `Options_FailClosedDefault` | New `SpendGuardOptions` returns `Deny` for `OnSidecarUnavailable` |
| `Options_Validation_RejectsEmptyBudgetId` | `services.AddSpendGuardMiddleware` throws on `BudgetId == ""` |
| `SpendGuardDecisionDeniedException_Carries_ReasonCodes` | Exception serialises `reason_codes`, `matched_rule_ids`, `decision_id`, `audit_decision_event_id` |
| `Middleware_DenyOutcome_ShortCircuits` | Mocked sidecar returns Deny; `next()` never invoked |
| `Middleware_AllowOutcome_InvokesNext_And_EmitsPost` | Allow → `next()` invoked, `EmitTraceEvents` receives `LLM_CALL_POST` with real usage |
| `Middleware_NextThrows_ReleasesReservation` | `next()` throws → middleware calls Release; usage event NOT emitted |

### 2.2 Python (pytest + pytest-asyncio)

Same matrix, Python-named:

| Test | Asserts |
|------|---------|
| `test_token_estimator_openai_matches_tiktoken` | Default `claim_estimator` (`agent_framework_default_claim_estimator`) returns tiktoken-derived counts |
| `test_idempotency_key_stable` | `derive_idempotency_key` output reproducible |
| `test_options_fail_closed_default` | `SpendGuardMiddleware(on_sidecar_unavailable="deny")` is the default |
| `test_decision_denied_exception_carries_reason_codes` | `DecisionDenied` from middleware preserves all reason fields |
| `test_middleware_deny_short_circuits` | Mocked client returns deny → `call_next` not awaited |
| `test_middleware_allow_invokes_next_and_emits_post` | Allow path emits `LLM_CALL_POST` with real provider usage |
| `test_middleware_next_raises_releases_reservation` | Inner raises → reservation released |
| `test_run_context_required` | `current_run_context()` raises clear error when middleware called outside `run_context()` |
| `test_function_middleware_decorator` | `@spendguard_function_middleware` decorates a function-middleware callable |

## 3. Integration tests (slices 4 + 7)

A **sidecar stub** runs as a fixture: an in-process gRPC server implementing the subset of `SidecarAdapter` needed (Handshake, RequestDecision, EmitTraceEvents).

| Test | Both langs | Asserts |
|------|-----------|---------|
| `Middleware_Handshake_NegotiatesCapability` | yes | Handshake exchanges SDK version + runtime kind + capability mask |
| `Middleware_Allow_RealUsage_PropagatedToPost` | yes | Provider returns `usage.totalTokens = 137` → post event carries 137 |
| `Middleware_Deny_NoProviderCall` | yes | Deny outcome → MAF inner chat client mock receives 0 invocations |
| `Middleware_Degrade_AppliedAsApplyFailed` | yes | DEGRADE returned; parity with langchain integration — surfaced as APPLY_FAILED |
| `Middleware_RequireApproval_PropagatesPendingApproval` | yes | REQUIRE_APPROVAL → caller sees typed pending-approval exception |
| `Middleware_SidecarDown_FailClosed` | yes | Sidecar UDS unavailable + default options → exception raised, no provider call |
| `Middleware_SidecarDown_FailOpen_Allowed` | yes | Customer opts `OnSidecarUnavailable=Allow` → warning logged, call proceeds, no audit row |
| `Middleware_Replay_Idempotent` | yes | Same `(run_id, llm_call_id)` invoked twice → sidecar receives identical idempotency_key both times |
| `Middleware_ToolCallPre_Optin` | yes | `SpendGuardToolMiddleware` registered separately → `TOOL_CALL_PRE` decision before tool runs |
| `Middleware_With_AgtComposite_Python_Only` | Python | AGT composite evaluator wrapped inside MAF middleware → no double-counting (only one `LLM_CALL_PRE` per call) |

## 4. Demo regression tests (slice 8)

Each demo mode has a recorded provider fixture (HTTP cassette via `vcrpy` for Python, `WireMock.NET` for .NET) so CI runs hermetically.

| Demo mode | Verifies |
|-----------|---------|
| `maf_dotnet_real` | .NET console example builds; runs end-to-end; produces ≥1 `canonical_events` row per `psql` count; provider call recorded |
| `maf_python_real` | Python example runs; same canonical_events row count; same provider call cassette |
| `maf_python_with_agt` | AGT evaluator path runs through MAF middleware once; canonical_events row count = number of LLM calls (no doubling) |
| `maf_dotnet_deny` | Seeded `budgets.cents_remaining = 0`; .NET middleware throws `SpendGuardDecisionDeniedException`; provider cassette has 0 hits |
| `maf_python_deny` | Same, Python `DecisionDenied` raised |

## 5. Snapshot / golden files

- **.NET:** `Verify.Xunit` snapshots for:
  - Generated NuGet `.nuspec` metadata (semver pin proof).
  - Idempotency-key byte output for a fixed seed (cross-language consistency proof vs Python `derive_idempotency_key`).
- **Python:** `pytest-snapshot` for:
  - `DecisionDenied.__repr__` output.
  - Sidecar gRPC request payload bytes for a deterministic input.

## 6. Cross-language consistency tests (slice 7)

A single Python test (`test_dotnet_python_idempotency_parity.py`) loads the .NET test fixture's seed and re-derives the idempotency key in Python; bytes must match. Guards against silent drift between the two implementations.

## 7. Negative tests (R3 of review-standards)

- **N1.** Empty BudgetId rejected at DI registration.
- **N2.** Empty SocketPath rejected at DI registration.
- **N3.** Calling middleware before Handshake raises typed `HandshakeRequiredException` (.NET) / `HandshakeRequiredError` (Python).
- **N4.** Tenant ID mismatch between options and handshake response → typed `TenantMismatchException`.
- **N5.** Non-UTF-8 prompt content does not crash the estimator (clamped + audit warning).

## 8. CI gates

| Gate | Where | When |
|------|-------|------|
| `make sdk-dotnet-test` | `.github/workflows/dotnet.yml` | every PR touching `sdk/dotnet/**` or `proto/**` |
| `make sdk-python-test` | existing `.github/workflows/python.yml` | every PR touching `sdk/python/**` |
| `make demo-up DEMO_MODE=maf_dotnet_real` regression | `.github/workflows/demo-regression.yml` | nightly + before tag |
| `make demo-up DEMO_MODE=maf_python_real` regression | same | nightly + before tag |
| Cross-lang parity test | `python-test` job | every PR |
