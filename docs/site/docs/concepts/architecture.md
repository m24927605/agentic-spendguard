# 6-layer architecture

SpendGuard organizes its concerns into 6 primitive layers, executed
in this strict order on every decision:

```
T (Trace) → L (Ledger) → C (Contract) → D (Decision) → E (Evidence) → P (Proof)
```

| Layer | Responsibility | Key invariant |
|---|---|---|
| **T** Trace | Capture event identity (run_id, step_id, llm_call_id) | Every event has a globally-unique id |
| **L** Ledger | Atomic budget reservation + commit | Per-unit balance preserved every tx |
| **C** Contract | Hot-path policy evaluation | Decision in <5ms |
| **D** Decision | 8-stage transaction state machine | Stages 1-4 always atomic |
| **E** Evidence | Audit chain durability | No effect without audit row (§6.1) |
| **P** Proof | Per-event signing + verification | Cosign-signed bundles + Ed25519 events |

See `docs/contract-dsl-spec-v1alpha1.md` and
`docs/stage2-poc-topology-spec-v1alpha1.md` in the source repo for the
full specifications.
