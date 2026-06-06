# D07 — Microsoft Agent Framework (MAF) middleware — acceptance.md

> Status: Doc-first; lands before any implementation slice. Scope-lock document.
> Sibling docs: `design.md` (what we ship), `implementation.md` (slice plan), `tests.md` (test pyramid), `review-standards.md` (per-slice gate).
> Audience: project owner (sign-off), implementers (target), reviewers (verification).

## 1. Acceptance philosophy

This doc defines the scope-locked answer to "what does **MAF middleware integration is done** look like?" Once accepted, criteria here cannot be expanded mid-implementation without explicit owner decision (`feedback_working_principles.md` Rule 3). Per build plan §3, every criterion must be runnable in the current repo state by the `superpowers:code-reviewer` reviewer without privileged access. Criteria cite design.md goal IDs (G1–G5) and ADR IDs (ADR-001..008).

## 2. Functional acceptance (maps to design.md G1–G5)

- **F1 (G1, drop-in, .NET).** Enable SpendGuard on an MAF .NET agent with at most three changes:
  1. `dotnet add package Spendguard.AgentFramework`.
  2. `services.AddSpendGuardMiddleware(opts => { opts.BudgetId = ...; opts.WindowInstanceId = ...; opts.TenantId = ...; });`.
  3. `AgentBuilder.UseMiddleware<SpendGuardChatMiddleware>()`.
  *Verified by:* the example in `examples/maf-dotnet/Program.cs` (slice 8) builds and runs with no other code edits, and `dotnet pack` produces a valid NuGet at `sdk/dotnet/dist/Spendguard.AgentFramework.<semver>.nupkg`.

- **F1' (G1, drop-in, Python).** Enable SpendGuard on an MAF Python agent with at most three changes:
  1. `pip install 'spendguard-sdk[agent-framework]'`.
  2. Construct `SpendGuardMiddleware(client=..., budget_id=..., window_instance_id=..., unit=..., pricing=...)`.
  3. `AgentBuilder().use_middleware(middleware)`.
  *Verified by:* `examples/maf-python/main.py` runs end-to-end with no other code edits.

- **F2 (G2, fail-closed).** When the sidecar UDS is unreachable:
  - **.NET:** `SpendGuardChatMiddleware` throws `SidecarUnavailableException` before invoking the chat client. MAF surfaces this to the calling agent loop; the inner `IChatClient` is never called.
  - **Python:** middleware raises `SidecarUnavailable`. MAF's agent loop surfaces it; `call_next` never awaited.
  Verified by `maf_dotnet_deny` and `maf_python_deny` demo sub-steps: counting provider cassette hits = 0, canonical_events row absent.

- **F3 (G3, both languages ship together).** Both surfaces work, by parallel code paths sharing the design doc:
  - **.NET:** `Spendguard.AgentFramework` published on NuGet (v0.5.x line, signed, deterministic).
  - **Python:** `spendguard.integrations.agent_framework` ships in `spendguard-sdk` v0.5.x via the new `[agent-framework]` extra.
  Verified by NuGet listing search + `pip install 'spendguard-sdk[agent-framework]==<version>'` smoke install.

- **F4 (G4, audit chain).** Every gated MAF call that reaches the provider produces exactly one `canonical_events` row joining the MAF `messageId` to `decision_id` to `llm_call_id`. No row when middleware denies.
  Verified by demo `maf_python_real` SQL count assertion: `SELECT count(*) FROM canonical_events WHERE event_type IN ('llm.call.pre','llm.call.post') AND tenant_id=<demo_tenant>` equals 2× number of LLM calls in the cassette.

- **F5 (G5, real-stack demo).** `DEMO_MODE=maf_dotnet_real`, `DEMO_MODE=maf_python_real`, `DEMO_MODE=maf_python_with_agt`, `DEMO_MODE=maf_dotnet_deny`, `DEMO_MODE=maf_python_deny` all defined as Makefile targets under `deploy/demo/Makefile` and run green in CI.
  Verified by `make demo-up DEMO_MODE=<mode>` exit code 0 + the per-mode post-flight check script.

## 3. Non-functional acceptance

- **NF1.** Both packages pass their respective linters: `dotnet format --verify-no-changes` and `ruff check` + `mypy --strict` for the new Python module.
- **NF2.** SBOM coverage: `Spendguard.AgentFramework` SBOM includes `Microsoft.Agents.Framework`, `Grpc.Net.Client`, `SharpToken`. Python extra updates `LICENSE_NOTICES.md` to include `agent-framework` package metadata.
- **NF3.** Security scan: `dotnet list package --vulnerable` clean; `pip-audit --strict` clean.
- **NF4.** Conformance: middleware passes the existing SpendGuard adapter conformance suite (`tests/conformance/adapter_conformance.py`) for both languages. The conformance suite already exists for langchain / openai_agents / pydantic_ai / agt; D07 extends it with two new runs.

## 4. Distribution acceptance

- **D1.** `Spendguard.AgentFramework` available on `nuget.org` (or staged on internal feed for v0.5.x prerelease; staged-only acceptable for first release if the README documents the planned promotion).
- **D2.** `spendguard-sdk` PyPI release notes mention the new `[agent-framework]` extra explicitly.
- **D3.** `README.md` `## Adapter integrations` table gets a row for **Microsoft Agent Framework (MAF)** with the AGT-vs-MAF callout from design.md §1.3.
- **D4.** `docs/site/docs/integrations/microsoft-agent-framework.md` user guide page exists, covers both languages, lists every option, includes a "MAF middleware vs AGT integration: which one?" subsection.
- **D5.** `CHANGELOG.md` entries for both packages, with reference to ADR IDs.

## 5. ADR enforcement

Every ADR in design.md §4 must be preserved by the merged code:

| ADR | Enforcement check |
|-----|-------------------|
| ADR-001 (two langs, one design) | design.md cited from both packages' README |
| ADR-002 (LLM-boundary, tool optional) | `SpendGuardToolMiddleware` separate class, not in default DI registration |
| ADR-003 (no proto changes) | `git diff main -- proto/` shows no changes touching adapter.proto |
| ADR-004 (default estimator) | .NET uses `SharpToken`; Python reuses tiktoken/tokenizers core dep |
| ADR-005 (fail-closed default) | Unit test `Options_FailClosedDefault` passes |
| ADR-006 (coexists with AGT) | `maf_python_with_agt` demo runs green; doc subsection ships |
| ADR-007 (replay safety) | Cross-lang parity test passes |
| ADR-008 (.NET versioning) | NuGet version string matches `spendguard-sdk` PyPI string at release time |

## 6. R5 reviewer self-check

The reviewer (`superpowers:code-reviewer`) must be able to verify every criterion above by reading the diff plus running the local commands listed in §2 and §3 — no external state, no credentials. If a slice ships code that requires an external NuGet publish to verify F3, the slice may stage acceptance with a TODO referencing the explicit publish step; final deliverable acceptance requires the publish to have happened.

## 7. Out of scope (explicit non-acceptance)

- SK or AutoGen filter parity — both are maintenance upstream; if a user requests SK coverage, point them at MAF migration.
- Azure OpenAI quota integration — orthogonal.
- Multi-region sidecar failover — out of scope per design NG.
- .NET Framework 4.x backport — out of scope per design NG.

## 8. Definition of done

D07 is **done** when all F1..F5, NF1..NF4, D1..D5, every ADR enforcement check in §5, and the R5 self-check in §6 hold against the merged diff on `main`, AND:

- Memory entry `project_coverage_D07_shipped.md` is written.
- `README.md` integration table row is present.
- Both demo modes are visible in `deploy/demo/Makefile`.
- No open GH issue blocks shipping (any residuals are tracked as new issues per the build plan).
