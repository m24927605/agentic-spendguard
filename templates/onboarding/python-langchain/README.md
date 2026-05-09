# SpendGuard onboarding — Python + LangChain (Phase 5 S20)

Goal: a design partner connects ONE real workflow to SpendGuard
without learning the whole platform first.

Estimated time: half a day. End state: your workflow shows
`STOP` (hard cap), `REQUIRE_APPROVAL` (soft cap), and `CONTINUE`
decisions in the SpendGuard dashboard.

## Prerequisites

- Existing Postgres instance (managed RDS / CloudSQL / etc.).
- A k8s cluster (kind, EKS, GKE, AKS — chart works on all four).
- An OIDC provider (Entra ID, Auth0, Okta, generic). Demo profile
  may use static tokens.
- `helm` v3.12+ and `kubectl` v1.27+.
- One of: `langchain >= 0.1` OR `pydantic-ai >= 0.0.5`.

## Files in this template

| File                       | Purpose                                                  |
|----------------------------|----------------------------------------------------------|
| `contract.yaml.tmpl`       | Contract DSL bundle; defines budgets + STOP/APPROVE rules |
| `budget.env.tmpl`          | Bootstrap params for `make onboard-bootstrap`            |
| `helm-values.yaml.tmpl`    | Minimal Helm values; production-shape defaults           |
| `sdk_adapter.py`           | Python wrapper over the sidecar UDS RPC                  |

## Step-by-step walkthrough

### 1. Generate identifiers

```bash
mkdir -p ~/spendguard-onboarding && cd ~/spendguard-onboarding
cp $SPENDGUARD_REPO/templates/onboarding/python-langchain/* ./
mv contract.yaml.tmpl contract.yaml
mv budget.env.tmpl    budget.env
mv helm-values.yaml.tmpl helm-values.yaml

# Tenant + budget IDs (UUID v4).
TENANT_ID=$(uuidgen)
BUDGET_ID=$(uuidgen)
WINDOW_INSTANCE_ID=$(uuidgen)
UNIT_ID=$(uuidgen)
BUNDLE_ID=$(uuidgen)
FENCING_SCOPE_ID=$(uuidgen)
WEBHOOK_FENCING_SCOPE_ID=$(uuidgen)
TTL_SWEEPER_FENCING_SCOPE_ID=$(uuidgen)
NOW_ISO=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
```

### 2. Resolve placeholders

```bash
sed -i.bak \
  -e "s|__TENANT_UUID_V4__|$TENANT_ID|g" \
  -e "s|__BUDGET_ID_UUID_V7__|$BUDGET_ID|g" \
  -e "s|__BUNDLE_ID_UUID_V7__|$BUNDLE_ID|g" \
  -e "s|__FENCING_SCOPE_UUID_V7__|$FENCING_SCOPE_ID|g" \
  -e "s|__WEBHOOK_FENCING_SCOPE_UUID_V7__|$WEBHOOK_FENCING_SCOPE_ID|g" \
  -e "s|__TTL_SWEEPER_FENCING_SCOPE_UUID_V7__|$TTL_SWEEPER_FENCING_SCOPE_ID|g" \
  -e "s|__ISO_8601_TIMESTAMP__|$NOW_ISO|g" \
  -e "s|__OWNER_TEAM__|your-team|g" \
  -e "s|__YOUR_WORKFLOW_NAME__|onboarding-workflow|g" \
  -e "s|__REGION__|us-west-2|g" \
  contract.yaml budget.env helm-values.yaml
```

Open each file and replace any remaining `__PLACEHOLDER__`s
(database URLs, OIDC issuer, etc.) by hand. The template refuses
to bundle while placeholders remain.

### 3. Provision tenant + budget via Control Plane

```bash
source budget.env
curl -X POST "$CONTROL_PLANE_URL/v1/tenants" \
  -H "Authorization: Bearer $ADMIN_BEARER_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{
        \"name\": \"$TENANT_NAME\",
        \"opening_deposit_atomic\": \"$OPENING_DEPOSIT_ATOMIC\",
        \"budget_unit_kind\": \"$BUDGET_UNIT_KIND\"
      }"
```

The response includes the actual budget_id + window_instance_id +
unit_id values — paste them back into `contract.yaml` if they
differ from the ones you generated in step 1. Future versions of
the template will round-trip these automatically.

### 4. Build the contract bundle

```bash
make -C $SPENDGUARD_REPO/sdk/python onboard-bundle \
    CONTRACT_YAML=$PWD/contract.yaml \
    OUT=$PWD/contract-bundle.tgz
```

The output is a signed tarball that the sidecar reads at startup.
Upload it to the cluster's `spendguard-bundles` Secret.

### 5. Install the chart

```bash
helm install spendguard $SPENDGUARD_REPO/charts/spendguard \
  -f helm-values.yaml \
  --create-namespace --namespace spendguard
```

Wait for pods to reach Ready (~60s). Verify:

```bash
kubectl -n spendguard get pods
kubectl -n spendguard logs -l app.kubernetes.io/component=sidecar | head -30
```

Expected log lines:
- `S6: producer signer initialized`
- `S4: fencing scope acquired via Ledger.AcquireFencingLease`
- `S22: fail-policy matrix initialized`

### 6. Wire your app

Mount the UDS socket into your app pod:

```yaml
volumes:
  - name: spendguard-uds
    hostPath:
      path: /var/run/spendguard
      type: DirectoryOrCreate
```

In your app code, replace your existing LLM client with the
`sdk_adapter.py` wrapper (or adapt the pattern to your framework).
The wrapper handles all three decision paths.

### 7. Smoke test

Run the included `sdk_adapter.py` directly:

```bash
SPENDGUARD_TENANT_ID=$TENANT_ID \
SPENDGUARD_BUDGET_ID=$BUDGET_ID \
SPENDGUARD_WINDOW_INSTANCE_ID=$WINDOW_INSTANCE_ID \
SPENDGUARD_UNIT_ID=$UNIT_ID \
SPENDGUARD_PRICING_VERSION=$(query_for_latest_pricing_version) \
SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX=$(query_for_hash_hex) \
python sdk_adapter.py
```

Expected output:
- One `echo: hello world` line (CONTINUE).
- One `awaiting approval: ...` line (REQUIRE_APPROVAL — soft cap).
- One `budget exhausted: ...` line (STOP — hard cap).

In the dashboard, you should see three `audit.decision` rows
with `decision_kind` values `continue` / `require_approval` /
`stop` respectively.

## Fail policy notes (S22)

Production profile defaults to fail-closed for every dependency.
Operators who want to allow LLM calls when (e.g.) pricing is
temporarily stale set `failPolicy.overrides` in `helm-values.yaml`:

```yaml
failPolicy:
  overrides: |
    {
      "_version": "2026-q3",
      "_acknowledge_risk_of_fail_open": true,
      "pricing": { "non_monetary_tool": "fail_open_with_marker" }
    }
```

Monetary workflows are fail-closed at parse time — no exceptions.

## Retention notes (S19)

The contract template sets `retain_audit_chain_for_days: 365`
and `redact_prompts_after_days: 30`. S19 (when shipped) wires
these into the storage layer. Today they're informational.

## Rollback / removal

To remove SpendGuard from your cluster:

```bash
helm uninstall spendguard --namespace spendguard
```

This stops all pods. The Postgres data (audit_outbox, canonical
events, ledger transactions) STAYS — operators retain the audit
chain even after uninstall per spec invariant. To purge data:

```bash
psql $LEDGER_URL    -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
psql $CANONICAL_URL -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
```

⚠️ **DESTRUCTIVE**. Audit data is permanently lost. Don't run
this in production without operator + compliance sign-off.

To remove the SDK from your app code:

1. Replace `call_with_spendguard(...)` calls with direct
   LLM calls.
2. Drop the `spendguard` import.
3. Remove the UDS volume mount from your pod spec.

The audit_outbox rows for past decisions stay queryable via the
S9 export endpoint until you DROP SCHEMA above.

## Troubleshooting

| Symptom                                    | Likely cause                                                            |
|--------------------------------------------|------------------------------------------------------------------------|
| `S4: acquire fencing lease at startup`     | Seed scope row missing — re-run migrations or pre-seed `fencing_scopes` |
| `S6: build signer from ... env`            | `signing.existingSecret` Secret missing or wrong format                 |
| `S22: signing.profile=production requires...` | Helm fail-gate; flip `signing.strictVerification: true`              |
| Sidecar healthy but app gets 503           | Adapter UDS path mismatch — verify volume mounts                        |

For more, see:
- `docs/site/docs/operations/multi-pod.md` (S5)
- `docs/site/docs/roadmap/ga-hardening-progress.md` (S1-S22 status)
