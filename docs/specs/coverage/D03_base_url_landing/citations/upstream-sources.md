# D03 — Upstream citation sources

> Status: COV_03 (Slice 1) seed. PDF snapshots land in SLICE 2 per
> `tests.md` §3.3. This file is the audit trail for every "Exact value"
> string used in the landing page matrix and the per-tool sections.
>
> Verification rule (`design.md` §3.4): the env var name / config key
> printed in the landing matrix must appear **verbatim** on the cited
> upstream docs URL on the slice's merge date. If the upstream page
> changes between spec time and slice time, the slice cannot merge
> without the per-row update.

---

## Citation table

Each row links the matrix row in `src/content/docs/docs/drop-in/index.mdx`
to the upstream URL we sourced the "Exact value" string from.

| # | Tool | Matrix "Exact value" string | Upstream docs URL | Citation evidence type |
|---|------|-----------------------------|-------------------|------------------------|
| 1 | LiteLLM (proxy mode) | `OPENAI_API_BASE=http://localhost:9000/v1` | https://docs.litellm.ai/docs/proxy/config_settings | Env var literal in proxy config settings reference |
| 2 | Aider | `OPENAI_API_BASE=http://localhost:9000/v1` | https://aider.chat/docs/llms/openai-compat.html | Env var literal in "OpenAI compatible APIs" guide |
| 3 | Continue | `apiBase: http://localhost:9000/v1` | https://docs.continue.dev/customization/models | YAML key `apiBase` in `config.yaml` model block |
| 4 | Cline / Roo Code (BYOK) | UI: Custom OpenAI provider, Base URL field | https://docs.cline.bot/getting-started/byok | UI walkthrough — manual review, PDF snapshot due SLICE 2 |
| 5 | OpenHands (BYOK) | UI: LLM custom endpoint (Settings → LLM → Custom Provider) | https://docs.all-hands.dev/usage/llms/custom-llm-configs | UI walkthrough — manual review, PDF snapshot due SLICE 2 |
| 6 | Goose | `OPENAI_HOST=http://localhost:9000` | https://block.github.io/goose/docs/getting-started/installation | Env var literal in installation guide |
| 7 | Zed AI | `api_url = "http://localhost:9000/v1"` | https://zed.dev/docs/ai/configuration | TOML key `api_url` in `settings.json`/`settings.toml` |
| 8 | GitHub Copilot CLI (BYOK) | `COPILOT_PROVIDER_BASE_URL=http://localhost:9000/v1` | https://docs.github.com/en/copilot/github-copilot-cli/using-byok | Env var literal in BYOK reference (GA 2026-04-07) |
| 9 | Tabnine Enterprise | Admin UI: BYO LLM endpoint URL | https://docs.tabnine.com/main/software-configuration/connect-custom-llm | Admin UI walkthrough — manual review, PDF snapshot due SLICE 2 |
| 10 | AnythingLLM | Admin UI: Custom OpenAI-compatible Base URL | https://docs.anythingllm.com/llm-configuration/custom-openai-base-url | Admin UI walkthrough — manual review, PDF snapshot due SLICE 2 |
| 11 | LobeChat | UI: Custom base URL (Settings → Language Model) | https://lobehub.com/docs/self-hosting/usage/byok | UI walkthrough — manual review, PDF snapshot due SLICE 2 |
| 12 | Cody self-hosted Enterprise | Sourcegraph site-config relay endpoint | https://sourcegraph.com/docs/cody/clients/install-vscode | Config walkthrough — manual review, PDF snapshot due SLICE 2 |
| 13 | Augment (BYOK) | UI: LLM custom endpoint | https://docs.augmentcode.com/setup/byok | UI walkthrough — manual review, PDF snapshot due SLICE 2 |
| 14 | Dify | Plugin manifest: custom Model Provider plugin | https://docs.dify.ai/plugins/model-provider | Plugin manifest reference — manual review, PDF snapshot due SLICE 2 |

Row 15 of the landing matrix (CrewAI Studio via LiteLLM) re-uses the
LiteLLM citation; no separate upstream source is required.

---

## How to refresh this file

1. Open the upstream URL in the citation row.
2. Search for the literal "Exact value" string (env var name, config
   key, or canonical UI step text).
3. If verbatim match: the citation is current — record the verification
   date in the slice's review log under `review-logs/COV_NN_<short>.md`.
4. If no verbatim match: open a `docs-link-drift` issue (per
   `review-standards.md` §2.4 S2) and update the matrix row + this file
   in the same slice.

PDF snapshots for the "manual review" rows (4, 5, 9, 10, 11, 12, 13, 14)
are captured under
`docs/specs/coverage/D03_base_url_landing/citations/snapshots/` in
SLICE 2 (see `tests.md` §3.3).
