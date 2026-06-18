<!--
Thanks for contributing to SpendGuard! Keep PRs focused on one logical change.
Security issues must NOT be filed here — see SECURITY.md.
-->

## What and why

<!-- What does this change do, and why? Link any related issue (e.g. Closes #123). -->

## Type of change

- [ ] Bug fix
- [ ] Feature
- [ ] Docs
- [ ] Refactor / chore

## Impact checklist

- [ ] **Fail-closed preserved** — no path can turn a `DENY` into an `ALLOW`.
- [ ] Touches `proto/` or a `migrations/` tree? If yes, the change is
      **additive** (new migration / backwards-compatible proto) and an issue
      was opened first.
- [ ] The affected component builds (`cargo check --tests` / package typecheck).
- [ ] Relevant tests added or updated.
- [ ] If a core flow or integration changed, its `DEMO_MODE` gate passes.
- [ ] Docs updated (README / `docs/` / service `README.md`) where needed.

## Verification

<!-- Commands you ran and their results (build, tests, demo mode). -->
