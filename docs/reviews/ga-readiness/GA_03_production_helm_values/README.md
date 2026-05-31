# GA_03 Production Helm Values Evidence

Date: 2026-05-31
Branch: `ga/GA_03_production_helm_values`
Tested implementation head: `670bfefa8cffee22e6404dfcbcbbba300cb21d8b`

## Commands

```bash
scripts/release/validate-production-helm-values.sh
helm template spendguard charts/spendguard --set chart.profile=demo
helm template spendguard charts/spendguard -f charts/spendguard/values-production.example.yaml
rg -n 'postgres(ql)?://|DATABASE_URL:|password:|api[_-]?key:' charts/spendguard/values-production.example.yaml
```

## Result

- Production values example exists: PASS.
- Production values render with `chart.profile=production`: PASS.
- Validator confirms database URLs are Secret refs only: PASS.
- Validator confirms Strategy C per-tenant SVID Certificate URI SAN coverage: PASS.
- Validator confirms NetworkPolicy renders: PASS.
- Validator confirms effective UID 65532, read-only root filesystem, no privilege escalation, and `capabilities.drop=[ALL]`: PASS.
- Plaintext database URL negative test: PASS, failed closed.
- Missing Strategy C SVID binding negative test: PASS, failed closed.
- Disabled security context negative test: PASS, failed closed.
- Demo Helm render: PASS.
- Production example Helm render: PASS.
- Plaintext credential grep on production example: PASS, no matches.

## Adversarial Review

Pending codex adversarial review rounds.
