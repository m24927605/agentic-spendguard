# spendguard-endpoint-catalog

Atomic-update endpoint catalog for the Agentic SpendGuard Phase 1 first-customer
(K8s SaaS-managed) POC.

Sidecars discover ledger / canonical_ingest / bundle_registry / regional CA
endpoints by:

1. Pulling the signed `manifest.json` (Cache-Control: no-store; max
   `manifest_validity_seconds` since `issued_at`).
2. Following `current_catalog_url` to the immutable versioned catalog
   object (24h CDN-cacheable).
3. Subscribing optionally to the SSE invalidation channel for prompt
   refresh (best-effort hint; pull is the correctness path).

## Spec map

- Sidecar §8 (endpoint discovery)
- Sidecar §7 (signed fail-safe manifest, critical_revocation_max_stale)
- Stage 2 §8.2.4 (atomic update model, no-store / immutable cache)
- Stage 2 §10.4 (POC Tier 3 deliverables)
- JSON schemas: `proto/spendguard/endpoint_catalog/v1/{manifest,catalog}.schema.json`

## Crate layout

```
src/
├── lib.rs                            module re-exports
├── main.rs                            HTTP server binary
├── bin/publish.rs                     publisher CLI (signs + atomically writes)
├── config.rs                          env-driven Config
├── server.rs                          axum Router + AppState
├── handlers/
│   ├── manifest.rs                    GET /v1/catalog/manifest (no-store)
│   ├── catalog.rs                     GET /v1/catalog/{version_id} (immutable)
│   └── sse.rs                         GET /v1/catalog/events (invalidation hint)
├── domain/
│   ├── manifest.rs                    Manifest / CatalogObject types
│   └── signing.rs                     ed25519 sign + verify (canonical JSON)
└── persistence/
    └── store.rs                       Store trait + Filesystem + S3 backends

Cargo.toml                             axum 0.7 + tokio + ed25519-dalek 2 + aws-sdk-s3 1
```

## Atomic update model

Publisher writes catalog version object FIRST, then atomically rewrites
the manifest pointer. Sidecars always fetch manifest BEFORE catalog, so
in-flight stale manifests still resolve to a valid (older) catalog
version and reach a consistent state on the next refresh.

- Filesystem backend: `tempfile + rename` for atomic manifest replacement
  on POSIX.
- S3 backend: PUT with strong read-your-writes consistency (us-west-2
  default) replaces the key atomically.

## Signing model

Manifest body is canonicalized (recursive key sort + no whitespace JSON)
then signed ed25519. Sidecars verify against a Helm-pinned root CA bundle
via the platform mTLS chain (per Sidecar §8 trust bootstrap).

The publisher CLI loads its signing key from PEM PKCS#8 file. Production
should source the key from a KMS / HSM via a thin gRPC signing service;
the CLI is suitable for POC + pre-prod environments only.

## SSE invalidation

`/v1/catalog/events` emits `event: catalog_invalidate` lines whenever a
publisher posts to `/v1/internal/notify-catalog-change` (NOT exposed in
this skeleton — wired in vertical slice expansion). Stream uses 20s
keep-alive (configurable) so sidecars (default 30s heartbeat timeout) do
NOT mark the connection stale prematurely.

SSE is best-effort. Sidecars fail-closed on `last_verified_critical_version_age
> manifest_validity_seconds` measured from the LAST manifest pull (not the
SSE socket state) per Sidecar §7.

## Publishing

```bash
cargo build --release --bin spendguard-catalog-publish

# Build a fresh catalog body from the JSON schema:
cat > /tmp/catalog-body.json <<JSON
{
  "issued_at": "2026-05-07T10:00:00Z",
  "ledger_endpoints": [
    {"endpoint_url": "https://ledger.us-west-2.spendguard.ai:50051",
     "region": "us-west-2",
     "consistency_capability": "single_writer_per_budget",
     "health": "HEALTHY"}
  ],
  "canonical_ingest_endpoints": [
    {"endpoint_url": "https://ci.us-west-2.spendguard.ai:50061",
     "region": "us-west-2",
     "ack_mode_capability": "remote_append_ack",
     "health": "HEALTHY"}
  ],
  "bundle_registry_endpoints": [
    {"endpoint_url": "https://ghcr.io/v2/spendguard/bundles",
     "registry_kind": "ghcr",
     "global_replicated": true,
     "namespaces": ["schema_bundle", "contract_bundle", "pricing_bundle"]}
  ],
  "regional_ca_endpoints": [
    {"endpoint_url": "https://ca.us-west-2.spendguard.ai",
     "region": "us-west-2",
     "issuer_protocol": "csr_external_issuer"}
  ]
}
JSON

SPENDGUARD_ENDPOINT_CATALOG_FILESYSTEM_ROOT=/var/lib/sg-catalog \
SPENDGUARD_ENDPOINT_CATALOG_REGION=us-west-2 \
SPENDGUARD_ENDPOINT_CATALOG_STORAGE_BACKEND=filesystem \
SPENDGUARD_ENDPOINT_CATALOG_SIGNING_KEY_PEM_PATH=/etc/sg/manifest.pem \
SPENDGUARD_ENDPOINT_CATALOG_SIGNING_KEY_ID=manifest-2026-Q2 \
  ./target/release/spendguard-catalog-publish /tmp/catalog-body.json
```

## What's deferred to vertical slice expansion

- KMS-backed signing service (replace PEM file)
- Internal notification endpoint POST /v1/internal/notify-catalog-change
  with mTLS-only access (gates broadcast on invalidation_tx)
- Per-tenant catalog overrides in production rollout pipeline
- SSE multi-replica fanout (Redis pubsub or NATS) for HA
- Rate limiting + abuse protection
- Chaos test suite (manifest sigverify failure, SSE drop+reconnect race,
  publisher concurrent run, S3 eventual consistency simulation)
