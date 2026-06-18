# GA 10 Issue Closure Evidence

Date: 2026-06-01

GA_10 closed only issues whose disposition was resolved, duplicate, or
historical-process-only in the triage report.

| Issue | Final state | Closure basis |
|---|---|---|
| #106 | CLOSED | HARDEN_05 explicit tokenizer rustls `aws_lc_rs` provider install. |
| #107 | CLOSED | HARDEN_05 removed unused tonic gzip feature flags. |
| #128 | CLOSED | SLICE_04 seed migration is schema-qualified with `public.tokenizer_versions`. |
| #138 | CLOSED | HARDEN_05 per-tenant DB-backed `count_tokens` quota. |
| #142 | CLOSED | HARDEN_05 tenant raw-text PII shadow opt-in default deny. |
| #144 | CLOSED | HARDEN_05 canonical ingest replay dedup covers tokenizer drift alert ingestion. |
| #153 | CLOSED | GA_09 production Helm/security scan proves no plaintext DB URL rendering. |
| #155 | CLOSED | Historical process issue recorded by GA_10 backlog triage. |
| #170 | CLOSED | Duplicate of #157; #157 remains the canonical post-GA implementation issue. |

Confirmation command:

```bash
for n in 106 107 128 138 142 144 153 155 170; do
  gh issue view "$n" --repo m24927605/agentic-spendguard \
    --json number,state,title \
    --jq '"#\(.number) \(.state) \(.title)"'
done
```

Output:

```text
#106 CLOSED Tokenizer mTLS rustls provider - explicit aws_lc_rs registration with explicit cipher suite config
#107 CLOSED Drop unused tonic gzip feature from tokenizer service to reduce attack surface
#128 CLOSED [SLICE_04 R2 Security F11] Migration 0050 INSERT not in DO block (schema-qualify or wrap)
#138 CLOSED [SLICE_05 R2 residual] Cool-down rate cap to prevent provider quota burn
#142 CLOSED [SLICE_05 R2 residual] PII flow per-tenant opt-in for raw_text shadow
#144 CLOSED [SLICE_05 R2 residual] Replay protection on tokenizer_drift_alert CloudEvent
#153 CLOSED SLICE_05 R3 N5 - tokenizer K8s/Helm plaintext config audit (workspace-wide)
#155 CLOSED SLICE_05 R3 N7 - commit 533e7ea scope-creep beyond subject (historical)
#170 CLOSED SLICE_06 R2 residual #14: Security F7 drift alert dedup (alias of #157 - close as duplicate)
```
