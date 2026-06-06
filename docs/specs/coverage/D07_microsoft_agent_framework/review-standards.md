# D07 — Microsoft Agent Framework (MAF) middleware — review-standards.md

> Status: Proposed. Sibling: `design.md`, `implementation.md`, `tests.md`, `acceptance.md`.
> Used by: `superpowers:code-reviewer` skill on every slice R1–R5 round.
> Aligns with: `docs/review-standards/staff-panel-arbitration-process.md` (R5 escalation), build plan §1.

## 1. Reviewer mandate

This document is the per-slice checklist for the R1–R5 reviewer. The reviewer **must** load this doc plus `acceptance.md` plus the current slice spec before generating findings. Findings outside the scope of these standards are reported as suggestions but not blockers.

## 2. Mandatory checks (every slice)

### 2.1 Design adherence (D)

- **D1.** Slice does not modify `proto/spendguard/sidecar_adapter/v1/adapter.proto` (ADR-003).
- **D2.** Slice does not introduce a SK-only or AutoGen-only code path (design NG3).
- **D3.** Slice does not introduce Azure OpenAI quota wiring (design NG4).
- **D4.** Slice respects the ADR-002 separation: LLM middleware ≠ tool middleware; they are distinct classes/registrations.
- **D5.** Slice respects ADR-005: fail-closed default; any `Allow` fallback path is explicitly opt-in with a logged warning.
- **D6.** Slice respects ADR-007: idempotency_key includes the full tuple from design.md §3.

### 2.2 Sidecar contract correctness (S)

- **S1.** Any new RequestDecision call uses the correct trigger (`LLM_CALL_PRE` or `TOOL_CALL_PRE`); no other triggers used in pre-gate paths.
- **S2.** `EmitTraceEvents` post-call emits a complete `usage` payload from the provider's response object (not a re-estimate of the prompt).
- **S3.** Handshake is mandatory before any other RPC (reviewer greps for `RequestDecision` not preceded by Handshake-success guard).
- **S4.** Release path: if `next()` / `call_next` raises, the reservation is released before the exception propagates.
- **S5.** Drain signal handling: the middleware does not start a new RequestDecision after receiving a `DrainSignal`.

### 2.3 Cross-language parity (P)

- **P1.** Idempotency-key derivation byte-identical between .NET and Python for the same seed (verified by cross-lang parity test).
- **P2.** Public-API naming parity: both languages expose the same option names (case-translated): `SocketPath`/`socket_path`, `BudgetId`/`budget_id`, `OnSidecarUnavailable`/`on_sidecar_unavailable`, `ClaimEstimator`/`claim_estimator`.
- **P3.** Exception type parity: `SpendGuardDecisionDeniedException` (.NET) ↔ `DecisionDenied` (Python); `SidecarUnavailableException` ↔ `SidecarUnavailable`; `HandshakeRequiredException` ↔ `HandshakeRequiredError`.
- **P4.** Both demos perform the same observable steps in the same order.

### 2.4 Security (Sec)

- **Sec1.** No credentials, API keys, sidecar tokens, or socket paths hard-coded in source or examples.
- **Sec2.** UDS socket file permissions inherited from the sidecar; the middleware does not chmod/chown the socket.
- **Sec3.** No allow-by-default fallback paths added.
- **Sec4.** `claim_estimator` execution is bounded (no unbounded loops over message arrays); reviewer flags any O(n²) over message count.
- **Sec5.** Logging redacts prompt/response content by default; only metadata and token counts are logged at INFO. Full content available only at DEBUG with explicit opt-in.
- **Sec6.** Dependency additions reviewed for license + maintenance status. `Microsoft.Agents.Framework` (MIT) + `Grpc.Net.Client` (Apache-2) + `SharpToken` (MIT) acceptable; any new dep needs a one-line rationale in the slice doc.

### 2.5 Testing (T)

- **T1.** Every public API has at least one unit test.
- **T2.** Every outcome branch (Allow / Deny / Degrade / RequireApproval) has at least one integration test.
- **T3.** The fail-closed default has its own explicit test (cannot be folded into another).
- **T4.** Cross-language parity test exists or is referenced from a previous slice that ships it.
- **T5.** Demo regression for the slice's surface runs in CI nightly.

### 2.6 Packaging (Pkg)

- **Pkg1.** `Spendguard.AgentFramework.csproj` `<TargetFrameworks>` exactly `netstandard2.1;net8.0`; no implicit `net6.0` etc.
- **Pkg2.** NuGet metadata fields all populated: `<PackageId>`, `<Authors>`, `<PackageLicenseExpression>Apache-2.0</PackageLicenseExpression>`, `<RepositoryUrl>`, `<RepositoryType>git</RepositoryType>`, `<PackageProjectUrl>`, `<PackageTags>spendguard;maf;agent-framework;budget</PackageTags>`, `<PackageReadmeFile>README.md</PackageReadmeFile>`.
- **Pkg3.** SourceLink + deterministic build enabled.
- **Pkg4.** Python `pyproject.toml` extras key spelled exactly `agent-framework` (hyphenated, lowercase).
- **Pkg5.** Python module path exactly `spendguard.integrations.agent_framework` (matches design.md §3.4).

### 2.7 Documentation (Doc)

- **Doc1.** Public-facing docstrings / XML doc-comments on every public type and option.
- **Doc2.** `docs/site/docs/integrations/microsoft-agent-framework.md` covers both languages.
- **Doc3.** README integrations table entry includes the AGT-vs-MAF callout from design.md §1.3.
- **Doc4.** CHANGELOG entries for both packages cite the ADR IDs they implement.

## 3. R1–R5 escalation specifics

| Round | Reviewer behavior |
|-------|-------------------|
| R1 | Full checklist applied. Findings categorized as Blocker / Major / Minor. |
| R2 | Re-run after implementer fix; only re-check findings from R1 plus any net-new regression. |
| R3 | Same as R2; reviewer adds an integration-perspective note if the fix touches cross-language parity. |
| R4 | Reviewer escalates any unresolved Blocker to Staff+ panel arbitration candidate. |
| R5 | If any Blocker still open → trigger Staff+ panel (Software Architect + Backend Architect + AI Engineer + Security Engineer + Senior Developer) per build plan §1.3. |

## 4. Slice-specific hot spots

| Slice | Highest-risk check |
|-------|--------------------|
| `COV_d07_01` (.NET skeleton) | Pkg1, Pkg2, Pkg3 |
| `COV_d07_02` (.NET UDS client) | S3 (handshake gate), Sec2 |
| `COV_d07_03` (.NET middleware) | S1, S4, D4, D5 |
| `COV_d07_04` (.NET tests) | T1–T5 |
| `COV_d07_05` (Python module skeleton) | Pkg4, Pkg5 |
| `COV_d07_06` (Python middleware) | S1, S4, D5, P2, P3 |
| `COV_d07_07` (Python tests) | T1–T5, P1 |
| `COV_d07_08` (demos + docs) | Doc1–Doc4, F4 (audit chain), `maf_python_with_agt` non-doubling |

## 5. Reviewer panel arbitration triggers

R5 panel arbitration triggered specifically by:

- Any unresolved Blocker after R4.
- Any cross-language parity failure that the implementer claims is "intentional drift" — Software Architect panelist must sign off.
- Any security finding (Sec1–Sec6) that the implementer disputes.
- Any acceptance gate (F1–F5, NF1–NF4, D1–D5) that the implementer claims is not applicable.

## 6. Review log convention

Per build plan §1.1: each round's findings get a markdown file under `docs/specs/coverage/D07_microsoft_agent_framework/review-logs/COV_d07_<slice>_R<n>.md` with sections: Blockers / Majors / Minors / Decisions / Diff hash. Reviewer summary at the bottom: "R<n> PASS" or "R<n> FAIL (N blockers, M majors)". Implementer pickup happens off the same file.

## 7. Out of scope for reviewer

- Performance tuning beyond the existing SDK's p99 budget.
- Visual / docs polish beyond Doc1–Doc4.
- Cross-deliverable concerns (e.g. how MAF interacts with D04 LangChain-TS) — those are tracked separately.
