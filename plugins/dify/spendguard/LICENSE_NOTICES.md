# SpendGuard Dify Plugin — Third-Party License Notices

The `spendguard` Dify Model Provider Plugin is licensed under
**Apache License 2.0**.

It bundles or runtime-depends on the following third-party software,
each governed by its own license:

## Bundled at install time (`requirements.txt`)

### `dify-plugin` (≥ 0.8.0, < 1.0.0)
- License: Apache 2.0
- Project: <https://github.com/langgenius/dify-plugin-sdks>
- Copyright © LangGenius, Inc.
- Notice: Dify Plugin SDK provides the `LargeLanguageModel` base class
  the SpendGuard plugin extends.

### `spendguard-sdk` (≥ 0.5.1)
- License: Apache 2.0
- Project: this repository (`sdk/python/`)
- Copyright © Agentic SpendGuard contributors

### `openai` (≥ 1.40, < 3.0)
- License: Apache 2.0
- Project: <https://github.com/openai/openai-python>
- Copyright © OpenAI
- Notice: Used by `_upstream/openai.py` for the OpenAI Chat
  Completions client.

### `anthropic` (≥ 0.40, < 1.0)
- License: MIT
- Project: <https://github.com/anthropics/anthropic-sdk-python>
- Copyright © Anthropic, PBC
- Notice: Used by `_upstream/anthropic.py` for the Anthropic Messages
  client; included as a baseline dep (not optional) because Dify's
  `.difypkg` packaging format pins all deps into a single bundle.

### `httpx` (≥ 0.27, < 1.0)
- License: BSD 3-Clause
- Project: <https://github.com/encode/httpx>
- Copyright © Encode OSS Ltd.
- Notice: Used by `spendguard_llm.py::_sidecar_tokenize` to call the
  sidecar `/v1/tokenize` HTTP companion.

## Transitive dependencies

The SDKs above pull in their own transitive deps (e.g. `pydantic`,
`anyio`, `typing-extensions`, `certifi`). Their licenses are
preserved in the wheel metadata under each transitive dependency's
`dist-info/` directory after `pip install`.

## Test-time only (`pytest`, `ruff`)

Test dependencies are NOT bundled into the `.difypkg`; they live in
`pyproject.toml` `[tool.pytest]` config only. Their licenses are not
distributed with the plugin.

## SpendGuard own license

The plugin source itself is Apache 2.0 — see the repo root
[`LICENSE`](../../../LICENSE) file.
