# POST_GA_10 Verification Evidence

Slice: `POST_GA_10_test_quality`
Branch: `post-ga/POST_GA_10_test_quality`
Issues: #109, #124

## Implementation Summary

- #109: extended `crates/spendguard-tokenizer/tests/fixtures/cross_check.json`
  with 7 adversarial UTF-8 fixture classes across `cl100k_base`,
  `o200k_base`, and `p50k_base`:
  - four-byte UTF-8
  - zero-width joiner sequence
  - RTL scripts
  - CJK + bidi mix
  - combining marks
  - BOM prefix
  - mixed noisy prompt
- #109: added
  `crates/spendguard-tokenizer/tests/fixtures/regenerate_openai_cross_check.py`
  so reviewers can re-derive OpenAI expected token ids/counts with
  canonical Python `tiktoken`.
- #109: strengthened `cross_check_fixture_schema.rs` so fixture metadata,
  token counts, Unicode coverage, and OpenAI vectors are deterministic
  tests instead of manual assertions.
- #124: replaced the oversimplified Llama notice with an operator-facing
  checklist covering `Built with Llama` attribution, the 700 million
  monthly active users threshold, and Meta Llama 3.1 Acceptable Use Policy
  obligations.
- #124: added README/spec pointers and `license_notices.rs` tests so the
  disclosure cannot silently regress.

## External License Sources

- Meta Llama 3.1 Community License:
  `https://github.com/meta-llama/llama-models/blob/main/models/llama3_1/LICENSE`
- Meta Llama 3.1 Acceptable Use Policy:
  `https://llama.meta.com/llama3_1/use-policy`

## Commands Run

```bash
python3 crates/spendguard-tokenizer/tests/fixtures/regenerate_openai_cross_check.py
```

Result:

```text
verified 24 OPENAI_TIKTOKEN fixture cases with Python tiktoken 0.12.0
```

```bash
cargo test --manifest-path crates/spendguard-tokenizer/Cargo.toml \
  --test cross_check_fixture_schema --test license_notices -- --nocapture
```

Result:

```text
cross_check_fixture_schema: 4 passed
license_notices: 3 passed
```

```bash
cargo fmt --manifest-path crates/spendguard-tokenizer/Cargo.toml --check
cargo build --manifest-path crates/spendguard-tokenizer/Cargo.toml
cargo test --manifest-path crates/spendguard-tokenizer/Cargo.toml
```

Result:

```text
cargo fmt --check: passed
cargo build: passed
spendguard-tokenizer tests:
  120 unit passed
  4 cross_check_fixture_schema passed
  3 license_notices passed
  15 seed_parity passed
  1 doc-test ignored
```

```bash
git diff --check
```

Result:

```text
passed
```
