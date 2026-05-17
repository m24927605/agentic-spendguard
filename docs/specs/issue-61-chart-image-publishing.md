# Issue #61 — Chart image publishing + kind Ready=8/8

> **Status**: spec v1 — design draft. Not yet implementation-locked. Codex r0 not yet run.
> **Source**: P47 follow-up. PR #60 closed the structural half of issue #47 (chart applies on kind with 7 Secrets + Postgres + helm install succeeds) but pods stay in `ImagePullBackOff` because chart images aren't published.
> **Goal**: chart pods reach `Ready` (not just "expected lifecycle phase") in the CI kind job within a 5-minute timeout.
> **Audience**: implementer closing #61 + the operator running `helm install --set chart.profile=production` against their own cluster.
> **Capability touched**: nothing in the runtime contract. Pure distribution + CI hardening.

---

## 1. Context — what gap this closes

`charts/spendguard/values.yaml` references six service images:

```
spendguard/ledger:0.1.0-alpha.1
spendguard/canonical-ingest:0.1.0-alpha.1
spendguard/sidecar:0.1.0-alpha.1
spendguard/webhook-receiver:0.1.0-alpha.1
spendguard/outbox-forwarder:0.1.0-alpha.1
spendguard/ttl-sweeper:0.1.0-alpha.1
```

None of these are pushed to docker.io or any other public registry. The kind validation script `scripts/helm-validate-kind.sh` proves the chart structure applies cleanly, but every pod hits `ImagePullBackOff` because there's nothing to pull. PR #60's pass criteria is intentionally permissive (`ImagePullBackOff` counts as "expected lifecycle phase") so the structural validation isn't masked by the image gap.

**Issue #47's original acceptance criterion #1 was**: "All 8 pods Ready under `helm install --set chart.profile=production` on kind". This is currently unsatisfiable. #61 closes that gap by publishing images and tightening the kind pass criteria.

**The 8 pods**: 6 chart service pods + 1 Postgres pod + 1 migrate Job pod. The migrate Job currently has a placeholder image (per `charts/spendguard/templates/migrations.yaml`); satisfying #61 means the migrate Job's image is also published or stubbed out cleanly.

---

## 2. Design

Three orthogonal pieces, pick all three to fully close #61:

1. **CI image-build workflow** that builds + pushes six images on tag push.
2. **`scripts/helm-validate-kind.sh` `BUILD_IMAGES=1` path** that builds images locally + `kind load docker-image` for dev iteration without registry access.
3. **kind job pass criteria tightening** from "expected lifecycle phase" to "Ready=6/6 within 5 minutes".

### Why ghcr.io not docker.io

- `ghcr.io/m24927605/spendguard-{ledger,...}` — GitHub Container Registry; auth via `GITHUB_TOKEN` in Actions; no separate account needed; supports `docker pull` anonymously for public repos.
- docker.io requires a Docker Hub account + separate auth in CI. Adds a moving piece for no benefit.

### Image tag scheme

- `0.1.0-alpha.1` — the alpha sentinel currently in values.yaml. Keep this as the chart's default for as long as the chart is in pre-1.0.
- `sha-<short>` — commit-pinned tag pushed on every main commit so PR `kind` jobs can pin to a known-good image.
- `latest-main` — moving tag pointing at the most recent main commit. Used by the kind script's default mode.

Chart's `values.yaml` keeps `0.1.0-alpha.1` as the operator-facing tag; CI overrides via `--set <svc>.image.tag=sha-<commit>` for PR validation.

### Multi-arch concerns

CI runs on Ubuntu amd64. kind runs on the same host so amd64-only images suffice for the validation rig. Real operators on arm64 (M-series Mac local cluster) need an arm64 image variant. Defer multi-arch to a separate spec; v1 of #61 ships amd64-only and documents the limitation.

---

## 3. Components

### 3.1 CI image-build workflow

**New file**: `.github/workflows/publish-images.yml`

Triggers:
- `push: tags: ['v*']` — official tagged release builds (e.g., `v0.1.0-alpha.1`)
- `push: branches: [main]` — moving `latest-main` + `sha-<short>` tags
- `workflow_dispatch` — manual build for ad-hoc testing

Jobs (matrixed over the six services):
1. Checkout
2. Set up Docker Buildx + login to ghcr.io via `GITHUB_TOKEN`
3. Build `deploy/demo/runtime/Dockerfile.<svc>` (the demo's Dockerfiles are the canonical source)
4. Tag as `ghcr.io/m24927605/spendguard-<svc>:<tag>` (matrix of tags per trigger)
5. Push

Cross-cutting:
- Use `actions/cache@v4` for Cargo registry + Rust target dir to reduce build time from ~5min cold to ~90s warm.
- Set image labels: `org.opencontainers.image.source`, `.revision`, `.version` per OCI spec so `docker inspect` reveals provenance.

### 3.2 `scripts/helm-validate-kind.sh` `BUILD_IMAGES=1` path

Add an early branch in the script:

```bash
if [ "${BUILD_IMAGES:-0}" = "1" ]; then
    log "BUILD_IMAGES=1: building chart service images via docker compose..."
    ( cd "${REPO_ROOT}/deploy/demo" && docker compose -f compose.yaml build \
        ledger canonical-ingest sidecar webhook-receiver \
        outbox-forwarder ttl-sweeper )

    # Re-tag from compose's `spendguard-demo-<svc>:latest` to the chart's
    # expected `spendguard/<svc>:0.1.0-alpha.1` tag.
    for svc in ledger canonical-ingest sidecar webhook-receiver outbox-forwarder ttl-sweeper; do
        docker tag "spendguard-demo-${svc}:latest" "spendguard/${svc}:0.1.0-alpha.1"
        kind load docker-image "spendguard/${svc}:0.1.0-alpha.1" --name "${CLUSTER_NAME}"
    done
    log "  images built + loaded into kind"
fi
```

Without `BUILD_IMAGES=1`, the script's existing behavior preserves (chart-structure validation only, pods stay in ImagePullBackOff).

### 3.3 kind job pass criteria tightening

**File**: `scripts/helm-validate-kind.sh` §8 pass criteria

Today (PR #60):
```
PHASE_OK_STATES = Running | ContainerCreating | PodInitializing | ImagePullBackOff | ErrImagePull
```

After #61 with `BUILD_IMAGES=1` (or CI using published images):
```
# Wait for chart pods to actually be Ready.
kubectl --context "${KUBECTL_CTX}" -n "${NAMESPACE}" wait \
    --for=condition=ready pod \
    -l app.kubernetes.io/name=spendguard \
    --timeout=5m

READY_COUNT=$(kubectl ... get pods -l app.kubernetes.io/name=spendguard \
    -o jsonpath='...containerStatuses[0].ready' | grep -c true)

if [ "$READY_COUNT" -ne 6 ]; then
    log "FAIL: expected 6/6 chart pods Ready, got ${READY_COUNT}"
    exit 1
fi
```

The "ImagePullBackOff is OK" carve-out from PR #60 is removed entirely; `CreateContainerConfigError` etc. remain hard failures.

### 3.4 CI workflow integration

**File**: `.github/workflows/helm-validate.yml` `kind` job

Add a step before `Run kind validation script`:

```yaml
- name: Log in to ghcr.io (for image pull)
  run: |
    echo "${{ secrets.GITHUB_TOKEN }}" | docker login ghcr.io -u ${{ github.actor }} --password-stdin

- name: Override chart image repos to point at ghcr.io
  run: |
    cat > /tmp/kind-image-override.yaml <<EOF
    ledger:
      image:
        repository: ghcr.io/m24927605/spendguard-ledger
        tag: latest-main
    canonicalIngest:
      image:
        repository: ghcr.io/m24927605/spendguard-canonical-ingest
        tag: latest-main
    # ... etc for all 6 services
    EOF
    echo "HELM_IMAGE_OVERRIDE_FILE=/tmp/kind-image-override.yaml" >> $GITHUB_ENV
```

Then `helm-validate-kind.sh` reads `HELM_IMAGE_OVERRIDE_FILE` as an extra `-f` argument to `helm upgrade --install`.

### 3.5 migrate Job image

**File**: `charts/spendguard/templates/migrations.yaml` + `values.yaml`

The migrate Job currently uses a placeholder. Either:
- (A) Build a `spendguard-migrations` image that bundles `golang-migrate` + the migration SQL files; push to ghcr.io alongside the other six.
- (B) Use an off-the-shelf `migrate/migrate:v4.17` image + mount the SQL files via a ConfigMap.

Option B is smaller surface area for v1. Defer A to a separate spec.

---

## 4. Implementation slices

### Slice 1 — image-build workflow scaffolding

Touch: `.github/workflows/publish-images.yml` (new).
Test: PR push triggers a dry-run build (no push) per service via matrix; all six matrix legs succeed; rendered manifest list contains the expected layers.
Codex: r1 review. Look for: buildx cache key collision; missing `permissions: packages: write`; ghcr.io auth scopes.

### Slice 2 — push on main + tag triggers

Touch: same workflow file; enable `push: branches: [main]` + `push: tags: ['v*']` triggers; verify against a no-op PR that the workflow doesn't fire on PRs (only main pushes).
Test: merge to main → 6 images appear at `ghcr.io/m24927605/spendguard-*:latest-main` and `:sha-<commit>`.
Codex: r1 review. Look for: race when two main pushes interleave (sha-<commit> is fine; latest-main is the race).

### Slice 3 — kind script BUILD_IMAGES=1 path

Touch: `scripts/helm-validate-kind.sh`.
Test: `BUILD_IMAGES=1 bash scripts/helm-validate-kind.sh` on local Docker → 6 images built + loaded into kind, helm install → 6/6 Ready within 5 minutes.
Codex: r1 review. Look for: re-tag idempotency on re-runs; `kind load` failure modes (insufficient kind disk).

### Slice 4 — kind CI job uses published images

Touch: `.github/workflows/helm-validate.yml` `kind` job — add ghcr login + image override.
Test: `workflow_dispatch` of the kind job → PASS with Ready=6/6.
Codex: r1 review. Look for: race between `publish-images.yml` finishing and `helm-validate.yml kind` job pulling — pin to `sha-<commit>` not `latest-main` for the same workflow_run.

### Slice 5 — tighten pass criteria

Touch: `scripts/helm-validate-kind.sh` §8.
Test: run with `BUILD_IMAGES=1` locally — PASS Ready=6/6; remove `BUILD_IMAGES` (no images) — script now FAILs (the carve-out is gone).
Codex: r1 review. Look for: 5-minute timeout adequacy for cold-start; explicit per-pod log dump on timeout for triage.

### Slice 6 — migrate Job

Touch: `charts/spendguard/templates/migrations.yaml` (use `migrate/migrate:v4.17` + ConfigMap-mounted SQL).
Test: kind validation includes a `Completed` migrate Job pod (8 total pods: 6 chart + 1 migrate + 1 Postgres).
Codex: r1 review. Look for: SQL files staying in sync across multiple services (ledger/canonical_ingest); init-order race with Postgres.

### Slice 7 — final sweep + memory + chart README update

Touch: chart README's "What this chart deploys" gets the migrate Job documented; `memory/project_overview.md` gets the issue-61 closure entry.

---

## 5. Test plan

### Unit / lint

- `helm lint charts/spendguard` — passes (existing).
- `docker buildx build` for each service from `deploy/demo/runtime/Dockerfile.<svc>` — produces a runnable image (existing demo proves this).

### Integration (kind)

- `BUILD_IMAGES=1 bash scripts/helm-validate-kind.sh` locally — Ready=6/6.
- `.github/workflows/helm-validate.yml` `kind` job on main — Ready=6/6.
- `.github/workflows/publish-images.yml` on main — 6 images at `ghcr.io/m24927605/spendguard-*:latest-main` + `:sha-<commit>`.

### Adversarial (codex)

- Stale `latest-main` tag: a PR's kind job pulls an image from an outdated workflow run → pin to `sha-<commit>` per workflow_run.
- Image-layer cache poisoning between PR builds → use `--no-cache` for tagged release builds; cache only PR builds.
- `kind load docker-image` race: two slices running in parallel might collide → script holds an exclusive lock on `${WORK_DIR}/kind-load.lock`.
- Pod resource exhaustion (memory limits in chart's resources) on a single-node kind cluster → script applies `resources.requests` halving via `--set` for the kind run only.

### Real-cluster (out of scope for this issue)

- Multi-arch (arm64) images — separate spec.
- Production cluster with cert-manager + KMS-backed signing — separate spec (the existing PR #36 KMS code path is what would be exercised).

---

## 6. Acceptance criteria

- [ ] `.github/workflows/publish-images.yml` exists; PRs build (no push) via matrix; main pushes publish 6 images to `ghcr.io/m24927605/spendguard-*:{sha-<commit>,latest-main}`; tag pushes publish to `:{<tag>,latest-main}`.
- [ ] `scripts/helm-validate-kind.sh` `BUILD_IMAGES=1` path builds locally + loads into kind + makes pods Ready.
- [ ] `.github/workflows/helm-validate.yml` `kind` job overrides chart image repos to `ghcr.io/...:sha-<commit>` and asserts Ready=6/6 within 5 minutes.
- [ ] Chart's migrate Job (per `templates/migrations.yaml`) completes successfully against the test Postgres in kind.
- [ ] `charts/spendguard/README.md` documents the published image namespace + migrate Job's image source.
- [ ] No regression in PR #60's template-level gates (helm lint + 4 template cases) — all still pass.
- [ ] Codex review reaches GREEN within 5 rounds per slice (matrix workflow → 5 rounds × ~6 slices ≈ tractable).

---

## 7. Code review standards (codex prompts)

**r1 adversarial focus per slice**:

Slice 1-2 (publish-images.yml):
- Are workflow permissions scoped to `packages: write` only (not blanket write)?
- Does `actions/checkout@v6` use `persist-credentials: false`?
- Buildx cache key uses what input? File hash of the Dockerfile alone, or the full source tree?
- What happens if two pushes to main happen concurrently? Tag race on `latest-main`?
- Are image labels populated with `org.opencontainers.image.{source,revision,version,licenses}`?

Slice 3 (BUILD_IMAGES=1):
- Idempotency on re-run: does `docker tag` complain if the tag already exists? (No, it just moves the tag.)
- `kind load docker-image` requires the cluster to exist BEFORE the load; the script's cluster-create block must run first.
- If 1 of 6 builds fails, do the rest still load? Or does `set -e` abort partway through and leave kind in a half-loaded state? Use a `for` loop with explicit error tracking.

Slice 4 (CI image override):
- Does the override correctly map values.yaml's nested keys (`canonicalIngest.image.repository` vs `canonical-ingest`)? Helm value-key conventions matter.
- Image-override file must NOT leak into chart defaults committed to the repo (CI-only).
- ghcr.io public-repo anonymous pull works for the PR's kind job? Or must it auth? Test with a deliberate token-less pull first.

Slice 5 (Ready=6/6):
- 5-minute timeout — is that achievable for the rust-based services on a single-node kind? Demo container startup time benchmarks: ledger ~3s, sidecar ~5s, others ~2s each. 6 × 5s = 30s; add Postgres healthy + migrate job + Rolling-update delays → 5 min is generous.
- What does failure look like for the operator? Dump pod logs + describe per pod (existing) is the right signal.

Slice 6 (migrate Job):
- ConfigMap mount of SQL files: do `services/ledger/migrations/` and `services/canonical_ingest/migrations/` SQL files stay in sync across versions? Helm chart bundles a specific snapshot — version drift is a real risk.
- Job restart policy on `Failed` — should be `Never` so a broken migration doesn't loop.
- Init-container readiness vs main-container readiness: migrate Job must complete BEFORE the chart services start, not in parallel.

**Staff escalation triggers** (per `auto-instrument-egress-proxy-spec.md` §14.1):
- Slice 4 r5 RED on auth — escalate to security (image auth + token leak surface).
- Slice 6 r5 RED on migration ordering — escalate to ledger-audit (migrations interact with audit-chain immutability triggers).

---

## 8. Demo verification

Per memory `feedback_demo_quality_gate.md`: every service must really run. Closure gate for this issue:

```bash
$ BUILD_IMAGES=1 bash scripts/helm-validate-kind.sh
[helm-validate-kind] openssl: /opt/homebrew/opt/openssl@3/bin/openssl
[helm-validate-kind] tar: gtar
[helm-validate-kind] BUILD_IMAGES=1: building chart service images...
[helm-validate-kind]   built spendguard/ledger:0.1.0-alpha.1
[helm-validate-kind]   built spendguard/canonical-ingest:0.1.0-alpha.1
[helm-validate-kind]   built spendguard/sidecar:0.1.0-alpha.1
[helm-validate-kind]   built spendguard/webhook-receiver:0.1.0-alpha.1
[helm-validate-kind]   built spendguard/outbox-forwarder:0.1.0-alpha.1
[helm-validate-kind]   built spendguard/ttl-sweeper:0.1.0-alpha.1
[helm-validate-kind]   loaded into kind
[helm-validate-kind] helm install ...
[helm-validate-kind] waiting for 6/6 chart pods Ready ...
pod/spendguard-spendguard-ledger Ready
pod/spendguard-spendguard-canonical-ingest Ready
pod/spendguard-spendguard-sidecar Ready
pod/spendguard-spendguard-webhook-receiver Ready
pod/spendguard-spendguard-outbox-forwarder Ready
pod/spendguard-spendguard-ttl-sweeper Ready
[helm-validate-kind] PASS — Ready=6/6 within 5m
```

---

## 9. Deferred items (NOT shipped in #61)

- Multi-arch (arm64) image variants for Mac M-series local clusters.
- Production-grade migrate Job image (option A above — `spendguard-migrations` with bundled tooling).
- cert-manager + workload-identity ClusterIssuer for true production-profile validation. Currently the kind script generates self-signed CA in band, which is structurally correct but not the production rotation path.
- KMS-backed signing exercised end-to-end. The code path landed in PR #36 (issue #8) but isn't exercised by the kind validation; that needs LocalStack KMS in the kind cluster.
- Endpoint-catalog publisher test-double that signs a real manifest the sidecar's `manifestVerifyKey` can verify. Currently the script generates the verify-key Secret but no live catalog. Sidecar boots in a degraded-but-running state.

---

## 10. References

- PR #60 — chart structural validation (closes #47 structurally)
- Issue #47 — original acceptance criterion #1 ("All 8 pods Ready")
- PR #36 — `feat(signing): real KmsSigner with aws-sdk-kms ECDSA_SHA_256` (closes #8)
- Memory `feedback_demo_quality_gate.md`
- Memory `feedback_codex_review.md`
- `charts/spendguard/values.yaml` — image repo/tag config
- `charts/spendguard/README.md` — operator-facing schema docs
- `scripts/helm-validate-kind.sh` — current kind validation rig
- `.github/workflows/helm-validate.yml` — template + kind CI gates
- `deploy/demo/runtime/Dockerfile.{ledger,canonical_ingest,sidecar,webhook_receiver,outbox_forwarder,ttl_sweeper}` — canonical Dockerfile sources
