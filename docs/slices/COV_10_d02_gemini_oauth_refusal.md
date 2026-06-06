# COV_10 — D02 Closed CLI install: Gemini OAuth free-tier refusal

> **Deliverable**: D02 Closed CLI install script + CA bootstrap
> **Slice**: 6 of 8 (S)
> **Spec set**: [`docs/specs/coverage/D02_closed_cli_install/`](../specs/coverage/D02_closed_cli_install/)

## Scope

Add the pre-install detector that refuses `spendguard install` when Gemini CLI is running in OAuth free-tier mode (no API key, no service account). Per design §3.5:

> Refuses `HTTPS_PROXY` setup when Gemini is in OAuth free-tier mode. Detection: `~/.gemini/oauth_creds.json` present AND `GEMINI_API_KEY` unset AND `GOOGLE_APPLICATION_CREDENTIALS` unset.

Rationale: Gemini's free-tier OAuth flow tokens are bound to Google's accounts.google.com — a forward HTTPS proxy with a self-signed CA breaks the OAuth refresh handshake, and the user has no API key to fall back on. Better to refuse cleanly with a pointed message than break the user's existing Gemini access.

Concretely:
- `services/cli/src/preflight/gemini.rs` — NEW:
  - `pub fn detect_gemini_oauth_freetier() -> GeminiPreflight`
  - `pub enum GeminiPreflight { NotInstalled, ApiKeyMode, ServiceAccountMode, OauthFreetierRefused }`
  - Detection:
    - Read `~/.gemini/oauth_creds.json` existence
    - Read `GEMINI_API_KEY` env var
    - Read `GOOGLE_APPLICATION_CREDENTIALS` env var
    - Logic: oauth_creds.json present AND both env vars unset → `OauthFreetierRefused`
- `services/cli/src/preflight/mod.rs` — NEW module-level preflight runner:
  - `pub fn run_preflight() -> Result<(), PreflightRefusal>`
  - Aggregates all preflight checks (Gemini for now; future: other tools)
  - On any refusal: returns a typed PreflightRefusal with actionable user message
- `services/cli/src/lib.rs`:
  - `install()` calls `preflight::run_preflight()` BEFORE any side effects (CA issuance, trust install, shell rc)
  - On Refusal: prints user-facing message + exits non-zero. Message format per design (cite design.md §3.5):
    ```
    Refusing to install: Gemini CLI is using OAuth free-tier authentication
    (~/.gemini/oauth_creds.json present, no GEMINI_API_KEY, no
    GOOGLE_APPLICATION_CREDENTIALS).
    
    SpendGuard's HTTPS proxy with a self-signed CA breaks Gemini's OAuth
    refresh handshake against accounts.google.com.
    
    To proceed:
      1. Set GEMINI_API_KEY (paid API tier), or
      2. Set GOOGLE_APPLICATION_CREDENTIALS (Vertex AI service account), or
      3. Use 'spendguard install --force-allow-gemini-oauth' to override
         (your Gemini CLI may stop working until you sign out + re-auth).
    ```
  - `install --force-allow-gemini-oauth` flag bypasses (per the refusal message)
- ≥8 unit tests:
  - oauth_creds.json present + both env vars unset → OauthFreetierRefused
  - oauth_creds.json present + GEMINI_API_KEY set → ApiKeyMode
  - oauth_creds.json present + GOOGLE_APPLICATION_CREDENTIALS set → ServiceAccountMode
  - oauth_creds.json absent → NotInstalled
  - Both env vars set → ApiKeyMode wins (first check)
  - `install` without --force-allow-gemini-oauth + OauthFreetierRefused → returns error
  - `install --force-allow-gemini-oauth` + OauthFreetierRefused → proceeds (logs warning)
  - Error message contains the 3-option recovery hint

## Files touched

| File | Why |
|------|-----|
| `services/cli/src/preflight/mod.rs` | NEW — preflight dispatcher |
| `services/cli/src/preflight/gemini.rs` | NEW — Gemini OAuth detector |
| `services/cli/src/lib.rs` | Wire preflight into install() |
| `services/cli/src/cli.rs` (or wherever args live) | `--force-allow-gemini-oauth` flag |

## Test/verification plan

1. `cargo build --manifest-path services/cli/Cargo.toml` clean
2. `cargo test --manifest-path services/cli/Cargo.toml --lib` — SLICE 5 baseline + ~8 new
3. SLICE 2-5 regression unchanged
4. `cargo fmt --check` + `cargo clippy -D warnings` clean
5. Smoke: create temp HOME with fake oauth_creds.json; run install dry; assert refusal with exit code

## Anti-scope

- No doctor impl — SLICE 7
- No uninstall workflow beyond signature — SLICE 8
- No other tool preflight checks (Gemini-specific for v1)
- No actually-touching oauth_creds.json (read-only existence check)

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D02_closed_cli_install/design.md) §3.5 (Gemini OAuth refusal line 77), §7 slice 6 row
- SLICE 5: [`COV_09_d02_shell_rc_emission.md`](COV_09_d02_shell_rc_emission.md)
