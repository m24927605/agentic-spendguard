# GA_03 Production Helm Values Evidence

Date: 2026-05-31
Branch: `ga/GA_03_production_helm_values`
Tested implementation head: `7591962623a41799d8b18877ba5c5eb39bf18617`

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
- HTTPS egress-proxy output predictor endpoint negative test: PASS, failed closed.
- Egress-proxy `DirectoryOrCreate` UDS hostPath negative test: PASS, failed closed.
- Unqualified migration image negative test: PASS, failed closed.
- Plaintext DB URL through Helm value override negative test: PASS, failed closed.
- Mutable first-party image tag negative test: PASS, failed closed.
- Empty Strategy C SVID issuer name negative test: PASS, failed closed.
- Dead `migrations.canonicalImage` value negative test: PASS, failed closed.
- Empty critical Secret reference negative test: PASS, failed closed.
- `calibrationReport` mutable image tag negative test: PASS, failed closed.
- `calibrationReport` empty tenant negative test: PASS, failed closed.
- Duplicate SVID `secretName` negative test: PASS, failed closed.
- Invalid SVID `secretName` negative test: PASS, failed closed.
- Disabled security context negative test: PASS, failed closed.
- Sidecar NetworkPolicy ports match rendered ledger/canonical/tokenizer/predictor/projector service ports: PASS.
- Production images render with `global.imageRegistry`: PASS.
- Digest-pinned image tag renders as `repo@sha256:<hex>`: PASS.
- Sidecar UDS hostPath uses pre-created `Directory`, not `DirectoryOrCreate`: PASS.
- Demo Helm render: PASS.
- Production example Helm render: PASS.
- Plaintext credential grep on production example: PASS, no matches.

## Adversarial Review

- R1: 3 Blockers, 1 Major, 1 Minor. Fixed sidecar NetworkPolicy ledger/canonical port swap, sidecar non-root hostPath writability posture, ignored `global.imageRegistry`, all-zero hash placeholder acceptance, and stale evidence.
- R2: 2 Blockers, 2 Majors, 1 Minor. Fixed egress-proxy `https://` predictor overclaim, egress-proxy UDS hostPath type, digest image rendering, migration image qualification, and stale evidence.
- R3: 0 Blockers, 4 Majors, 2 Minors. Fixed chart-level plaintext DB URL rejection, first-party image tag gates, Strategy C issuer required fields, migration image mutable-tag policy, dead `canonicalImage` config, and stale evidence.
- R4: 1 Blocker, 2 Majors, 1 Minor. Fixed non-DB critical Secret fail gates, calibrationReport production image/tenant validation, SVID secretName DNS/duplicate validation, and stale evidence.
