# GA_03 Production Helm Values Evidence

Date: 2026-05-31
Branch: `ga/GA_03_production_helm_values`
Tested implementation head: `2c6f5bad8a820c987fc94c6c8d1b369c87669569`

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
- All-zero hash placeholder negative test: PASS, failed closed.
- Disabled security context negative test: PASS, failed closed.
- Sidecar NetworkPolicy ports match rendered ledger/canonical/tokenizer/predictor/projector service ports: PASS.
- Production images render with `global.imageRegistry`: PASS.
- Sidecar UDS hostPath uses pre-created `Directory`, not `DirectoryOrCreate`: PASS.
- Demo Helm render: PASS.
- Production example Helm render: PASS.
- Plaintext credential grep on production example: PASS, no matches.

## Adversarial Review

- R1: 3 Blockers, 1 Major, 1 Minor. Fixed sidecar NetworkPolicy ledger/canonical port swap, sidecar non-root hostPath writability posture, ignored `global.imageRegistry`, all-zero hash placeholder acceptance, and stale evidence.
