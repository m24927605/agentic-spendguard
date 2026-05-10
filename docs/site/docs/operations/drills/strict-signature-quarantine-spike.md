# Drill: strict-signature quarantine spike

Quarterly drill. Validates the audit chain integrity guarantee
under signature failure: when a producer's signing key rotates
without the verifier's trust store updating in lockstep, the
canonical_ingest verifier MUST reject every affected row to
`audit_signature_quarantine` (in strict mode) or admit + bump
admit-counters (in non-strict mode), but in either case must
NEVER drop or silently re-encode the bytes.

This is the live counterpart to the unit tests in
`services/canonical_ingest/src/verifier.rs::tests::*` and the
metrics tests in `services/canonical_ingest/src/metrics.rs::tests`.

## What this drill exercises

- Strict-mode: alert A5 `SpendGuardCanonicalRejectsHigh` fires.
- Non-strict mode (PR #2 round 1 P2#3 fix in `eec0404`): the
  `unknown_key_admitted_total` and
  `invalid_signature_admitted_total` counters bump but rows still
  land in `canonical_events` so audit-chain isn't broken during a
  rolling key rotation.
- The S7 key registry (`signing_keys` + `signing_key_revocations`
  tables in canonical-ingest migrations 0008/0009) — quarantine
  reasons differentiate `key_expired` / `key_revoked` /
  `key_not_yet_valid` / `unknown_key` / `invalid_signature`.

## Symptoms (what on-call sees)

- Alert A5 `SpendGuardCanonicalRejectsHigh` firing.
- `audit_signature_quarantine` row count climbing.
- `canonical_events` count growing slower (or flat in strict
  mode).
- `audit_outbox.pending_forward = TRUE` count climbing —
  forwarder keeps re-attempting the same rejected rows.
- User-visible: NO immediate impact on producers (sidecar /
  ledger / webhook still write rows). Audit consumers see
  growing quarantine; compliance sees gap.

## First check

```bash
# 1. Quarantine breakdown by reason (Phase 5 S7 + S8 schema):
psql -h $CANONICAL_PG_HOST -U spendguard -d spendguard_canonical -c "
  SELECT reason, count(*), max(quarantined_at) AS most_recent
    FROM audit_signature_quarantine
   WHERE quarantined_at > now() - interval '1 hour'
   GROUP BY reason
   ORDER BY count DESC;
"

# 2. Which signing keys are involved?
psql -h $CANONICAL_PG_HOST -U spendguard -d spendguard_canonical -c "
  SELECT signing_key_id, count(*) AS quarantined_rows
    FROM audit_signature_quarantine
   WHERE quarantined_at > now() - interval '1 hour'
   GROUP BY signing_key_id
   ORDER BY count DESC;
"

# 3. Compare signing keys claimed by producers vs trust store:
psql -h $CANONICAL_PG_HOST -U spendguard -d spendguard_canonical -c "
  SELECT key_id, valid_from, valid_until, revoked_at IS NOT NULL AS is_revoked
    FROM signing_keys
   ORDER BY valid_from DESC
   LIMIT 10;
"

# 4. Strict mode check (different remediation for strict vs non-strict):
kubectl exec <canonical-ingest-pod> -- env | grep STRICT_SIGNATURES
# true  → strict (rows rejected); false → non-strict (admitted + counted)
```

## Mitigation (short-term unblock)

Route depends on which `reason` dominates step 1:

### `unknown_key` dominates

Producer is using a key the verifier doesn't recognise. Likely
cause: key rotation deployed to producers ahead of trust-store
update on canonical-ingest.

1. **Identify the new key** (step 2 + the producer's recent
   logs).
2. **Add it to the trust store**:
   ```bash
   kubectl edit secret spendguard-signing-trust-store
   # Append the new public key + valid_from window
   kubectl rollout restart deployment canonical-ingest
   ```
3. **Replay the quarantined rows**: PR #2 round 1 quarantine
   keeps the original bytes verbatim. After trust store update,
   manual re-ingest from `audit_signature_quarantine` table
   into `canonical_events` (S8-followup feature; today requires
   manual SQL).

### `invalid_signature` dominates

This is more serious — bytes don't match the claimed signature.
Possibilities:
- Producer code regression (signing the wrong canonical bytes)
- Active tampering on the wire (mTLS misconfiguration?)

1. **Halt the affected producer immediately** until root cause
   is known:
   ```bash
   kubectl scale deployment <producer-name> --replicas=0
   ```
2. **Diff the producer image vs known-good** for changes to
   canonical-form serialization.
3. **Do NOT drop or replay quarantine rows** until tampering is
   ruled out — the bytes are forensic evidence.

### `key_expired` / `key_revoked` dominates

S7 validity-window enforcement. Producer is signing with a key
past its `valid_until` or after `revoked_at`.

1. **Rotate the producer's signing material to a current key**.
2. **Audit the gap**: rows signed with the expired key in
   `valid_from`-to-`valid_until` window are still legitimate
   (signed by a then-valid key); rows signed AFTER the window
   represent a producer config bug.

## Escalation

- **5 minutes** sustained spike → page platform oncall.
- **15 minutes** without diagnosis → page sidecar/ledger team
  oncall (depending on which producer is affected).
- **`invalid_signature` >0 rows** → security team page
  immediately (potential tampering).
- **30+ minutes** sustained quarantine in strict mode →
  consider switching to non-strict temporarily (operator
  decision, requires Helm gate ack — this trades audit-chain
  completeness for availability while you fix the root cause).

## Rehearsal

```bash
# 1. Bring up demo with strict mode enabled (default for
# production profile).
make demo-up DEMO_MODE=invoice

# 2. Generate a few audit rows.
make demo-up DEMO_MODE=decision

# 3. Inject a "key rotation" scenario by replacing one
# producer's signing key WITHOUT updating the verifier's trust
# store. Easiest via re-running pki-init with a new key, then
# restarting the sidecar:
docker exec spendguard-pki-init /generate.sh --rotate-sidecar
docker restart spendguard-sidecar

# 4. Generate more audit traffic.
make demo-up DEMO_MODE=decision

# 5. Confirm quarantine row appears with reason='unknown_key'.
docker exec spendguard-postgres psql -U spendguard -d spendguard_canonical -c "
  SELECT reason, count(*) FROM audit_signature_quarantine GROUP BY reason;
"
# Expected: unknown_key reason with at least 1 row.

# 6. Mitigation rehearsal: update the trust store + restart
# canonical-ingest, then verify new rows land in canonical_events
# (old rows stay in quarantine for the manual replay step).

make demo-down
```

## Related

- L5 SLO definition: `docs/site/docs/operations/slos.md` row L5
- Alert: A5 `SpendGuardCanonicalRejectsHigh` in
  `deploy/observability/prometheus-rules.yaml`
- D3 in slos.md (signature-failure handling) — high-level version
- PR #2 round 1 commit `a4dea4b` — non-strict admit counters
- PR #2 round 7+8 commits `409c220`, `d019e94` — SP-side
  literal-pin relaxations that let real signed rows through
