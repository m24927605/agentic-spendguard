# SpendGuard Dify Plugin — Changelog

All notable changes to the `spendguard` Dify Model Provider Plugin.
Follows [Keep a Changelog](https://keepachangelog.com/) ordering.

## [1.0.0] — 2026-06-07

Initial public release.

### Added — SLICE 1–4 (shipped 2026-06-07, `17c35d7`)
- Plugin scaffold (`manifest.yaml`, `pyproject.toml`,
  `requirements.txt`, `provider/spendguard.yaml`,
  `models/llm/llm.yaml`).
- `SpendGuardLLM(LargeLanguageModel)` adapter with sync/async bridge
  (daemon-scoped event loop, NOT `asyncio.run()` per call).
- `_DifyReservation` delegate — composition over inheritance — owns
  reserve / commit_success / release_failure lifecycle.
- OpenAI non-streaming forwarder (`_upstream/openai.py`):
  per-call client (multi-workspace safety), `spendguard/` prefix
  strip, real-usage commit (INV-5), full `openai.*` exception
  translation to Dify `InvokeError` subclasses, no `openai_api_key`
  in logs (INV-6).
- `validate_credentials` install-time probe at the SDK boundary +
  deep sidecar roundtrip in the provider class.
- 35 unit tests covering INV-1 / INV-5 / INV-6 + multi-workspace
  safety + error translation table.

### Added — SLICE 5 (this release)
- Anthropic non-streaming forwarder (`_upstream/anthropic.py`):
  mirror structure of `openai.py`; system-message split into the
  top-level `system` param + filtered `messages` list; full
  `anthropic.*` exception translation (including 529 Overloaded →
  `InvokeServerUnavailableError`); per-vendor TextBlock content
  extraction; `max_tokens` floor (1024) when the Dify form omits it;
  drops unsupported OpenAI params (`frequency_penalty` /
  `presence_penalty`) silently.
- `get_num_tokens` routes through the sidecar `/v1/tokenize` HTTP
  companion when `SPENDGUARD_SIDECAR_HTTP_URL` is set; falls back
  to `chars/4` on any companion failure (unreachable, non-200,
  timeout, `httpx` missing).
- `requirements.txt` adds `anthropic>=0.40,<1.0` + `httpx>=0.27,<1.0`
  (both baseline deps — Dify plugin format has no extras concept).
- 15 new unit tests covering A01–A15 (4.1 / 4.2 / 4.3 / 4.4 / 4.5 /
  4.6 / 4.8 / 5.1 / 5.2 / 5.3).

### Added — SLICE 6 (this release)
- `_stream_generate` SSE proxy for both OpenAI and Anthropic.
  Reserve fires once before any upstream HTTP (INV-1 streaming);
  every upstream SSE event yields a Dify `LLMResultChunk`; the
  `_StreamingAccumulator` captures content + usage across chunks
  and fires `commit_success` at end-of-stream.
- OpenAI streaming sets `stream_options.include_usage=True`
  unconditionally so commit has real `prompt_tokens` /
  `completion_tokens` (review-standards 6.3).
- Anthropic streaming accumulates `input_tokens` from
  `message_start` and `output_tokens` from `message_delta`; final
  empty chunk surfaces the `stop_reason` as `finish_reason`.
- Estimator fallback (chars/4) when upstream omits usage entirely;
  WARN logged so operators can spot the case (review-standards 6.4).
- Mid-stream upstream errors → `release_failure(handle, exc)` →
  re-raise translated `InvokeError`; reservation cleanly released.
- Caller cancellation (`GeneratorExit` / `CancelledError`) routes
  to `release_failure` with the `CANCELLED` classification.
- 10 new unit tests covering S01–S08 (6.1 / 6.2 / 6.3 / 6.4 / 6.5 /
  6.6 / 6.7 / 6.8).

### Added — SLICE 7 (this release)
- `DEMO_MODE=dify_plugin_real` end-to-end demo
  (`deploy/demo/dify_plugin/docker-compose.yaml` overlay +
  `run_dify_plugin_demo.py` 3-step matrix +
  `verify_step_dify_plugin_real.sql` ledger-side gates).
- `Makefile` `dify_plugin_real` branch boots
  `postgres + sidecar + ledger + counting-stub + dify-plugin-runner`
  and drives ALLOW + DENY + STREAM through `SpendGuardLLM._invoke()`
  against the real sidecar.
- INV-2 strict-order proof: earliest reserve predates earliest
  outcome (verify SQL DO block).

### Added — SLICE 8 (this release)
- Public docs page
  [`docs/site-v2/.../integrations/dify.mdx`](https://agenticspendguard.dev/docs/integrations/dify/)
  with install / configuration / lifecycle / streaming /
  limitations / demo / troubleshooting sections.
- Root `README.md` adapter row + `make demo-up
  DEMO_MODE=dify_plugin_real` row.
- Astro sidebar entry under Adapter integrations.
- This `CHANGELOG.md` + `LICENSE_NOTICES.md`.

### Deviation notes
- `dify-plugin` SDK floor: spec design.md §5 / implementation.md §2
  pinned `dify-plugin>=0.2.0,<0.3.0`; live PyPI floor is 0.8+ which
  predates the v1 Model Provider Plugin contract that ships
  `LargeLanguageModel`. Pinned to `>=0.8.0,<1.0.0` in `requirements.txt`.
- Demo focuses on the SDK boundary, not the full Dify Workspace
  HTTP frontend. The Workspace chat-message routing is upstream of
  the plugin and out of scope for the plugin value proposition.
  See `deploy/demo/dify_plugin/docker-compose.yaml` rationale.
- Bedrock + Gemini upstream reserved for v1.1; the v1 form lists
  them for forward-compat but `build_upstream_client` raises
  `InvokeError("not supported in this plugin version")`.
- Dify plugin distribution uses `dify plugin pack` (`.difypkg` zip)
  not npm/PyPI; the GitHub release workflow at
  `.github/workflows/dify-plugin-publish.yml` packages and signs
  the bundle, then attaches it as a release asset and (when the
  marketplace OIDC config lands) pushes to the Dify plugin
  marketplace.

[1.0.0]: https://github.com/m24927605/agentic-spendguard/releases/tag/dify-plugin-v1.0.0
