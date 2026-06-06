# D01 — Envoy AI Gateway ExtProc Sidecar — Implementation

**Companion to:** [`design.md`](design.md)
**Code layout owner:** Backend Architect

---

## §1. Module layout

```
services/envoy_extproc/
├── Cargo.toml
├── build.rs                          # tonic-build for ext_proc.v3
├── proto/
│   └── envoy/                        # vendored Envoy ext_proc proto tree
│       └── service/ext_proc/v3/
│           └── external_processor.proto
├── src/
│   ├── main.rs                       # binary entry: TLS server + sidecar client init
│   ├── lib.rs                        # public re-exports for tests
│   ├── config.rs                     # SPENDGUARD_EXTPROC_* env reader
│   ├── proto.rs                      # tonic-build module mounts
│   ├── server.rs                     # impl ExternalProcessor for ExtProcServer
│   ├── stream.rs                     # per-HTTP-stream state machine
│   ├── translate/
│   │   ├── mod.rs
│   │   ├── request_phase.rs          # HeadersReq + BodyReq -> RequestDecision
│   │   ├── response_phase.rs         # BodyResp -> EmitTraceEvents(LLM_CALL_POST)
│   │   └── decision_map.rs           # DecisionResponse -> ExtProc CommonResponse
│   ├── sidecar_client.rs             # tonic mTLS client of SidecarAdapter
│   ├── tls.rs                        # rustls + SVID cert load (mirrors output_predictor)
│   └── metrics.rs                    # /metrics endpoint (Prometheus)
└── tests/
    ├── conformance/                  # SLICE 5 golden-fixture tests
    │   ├── envoy_v06_token_counting.yaml.json
    │   ├── envoy_v06_budget.yaml.json
    │   └── fixtures.rs
    ├── translate_request_phase.rs
    ├── translate_response_phase.rs
    ├── stream_lifecycle.rs
    └── tls_loopback.rs

crates/spendguard-provider-routing/       # SLICE 1 extraction
├── Cargo.toml
└── src/
    └── lib.rs                            # ProviderKind + ProviderConfig + ROUTING_TABLE moved here

charts/spendguard/templates/
└── envoy_extproc.yaml                    # SLICE 6

deploy/demo/
├── Makefile                              # +DEMO_MODE=envoy_extproc target
└── envoy_demo/
    ├── envoy.yaml                        # Envoy proxy config calling our ExtProc
    └── mock_upstream.py                  # static OpenAI/Anthropic responses
```

## §2. Crate `services/envoy_extproc/Cargo.toml`

```toml
[package]
name = "spendguard-envoy-extproc"
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"
publish = false

[dependencies]
# Sidecar IPC (mirrors services/egress_proxy/Cargo.toml).
tonic = { version = "0.12", features = ["tls", "tls-roots"] }
prost = "0.13"
prost-types = "0.13"
tokio = { version = "1.40", features = ["macros", "rt-multi-thread", "signal"] }
tokio-stream = "0.1"
tokio-rustls = "0.26"
rustls = "0.23"
rustls-pemfile = "2"
async-trait = "0.1"

# Re-used in-process:
spendguard-tokenizer = { path = "../../crates/spendguard-tokenizer" }
spendguard-provider-routing = { path = "../../crates/spendguard-provider-routing" }

# Sidecar proto crate (workspace member exclude, but importable as path dep).
spendguard-sidecar-adapter-proto = { path = "../sidecar/crates/proto-client" }

# Observability.
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
prometheus = "0.13"

# Errors.
thiserror = "1"
anyhow = "1"

# JSON for body parsing (chat/completions, messages).
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bytes = "1"
uuid = { version = "1", features = ["v7"] }

[build-dependencies]
tonic-build = "0.12"

[dev-dependencies]
hyper = { version = "1", features = ["full"] }
hyper-util = { version = "0.1", features = ["tokio"] }
tower = "0.5"
tempfile = "3"
pretty_assertions = "1"
```

## §3. SLICE 1 — `src/main.rs` skeleton

```rust
//! Envoy AI Gateway ExtProc sidecar — translates Envoy ExternalProcessor gRPC
//! calls into SpendGuard sidecar UDS adapter calls.
//!
//! Spec refs:
//!   - docs/specs/coverage/D01_envoy_extproc/design.md §3
//!   - Envoy AI Gateway v0.6 ExternalProcessor proto (vendored under proto/)

use std::net::SocketAddr;
use std::sync::Arc;

use tonic::transport::Server;
use tracing::info;

use spendguard_envoy_extproc::{
    config::Config,
    proto::envoy::service::ext_proc::v3::external_processor_server::ExternalProcessorServer,
    server::ExtProcService,
    sidecar_client::SidecarClient,
    tls,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let cfg = Config::from_env()?;
    info!(?cfg.bind_addr, "spendguard-envoy-extproc starting");

    // mTLS to sidecar (TCP, per design §3.3).
    let sidecar = SidecarClient::connect(&cfg).await?;
    sidecar.handshake().await?;

    let svc = ExtProcService::new(Arc::new(sidecar));

    let tls_cfg = tls::server_tls_config(&cfg)?;
    let addr: SocketAddr = cfg.bind_addr.parse()?;

    Server::builder()
        .tls_config(tls_cfg)?
        .add_service(ExternalProcessorServer::new(svc))
        .serve_with_shutdown(addr, async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;

    Ok(())
}
```

## §4. SLICE 1 — `crates/spendguard-provider-routing/src/lib.rs`

Move `ProviderKind`, `RequestShape`, `UsageMetrics`, `ProviderConfig`, `ROUTING_TABLE`, `route`, `resolve_model_id`, `resolve_tokenizer_kind` from [`services/egress_proxy/src/routing.rs`](../../../../services/egress_proxy/src/routing.rs) (lines 37-349) into the new crate. The egress_proxy keeps its `providers::*::extract_usage` private functions and re-exports the moved items via:

```rust
// services/egress_proxy/src/routing.rs (post-SLICE-1)
pub use spendguard_provider_routing::{
    ProviderConfig, ProviderKind, RequestShape, UsageMetrics,
    ROUTING_TABLE, route, resolve_model_id, resolve_tokenizer_kind,
};

// Local extractor functions stay in services/egress_proxy/src/providers/ and
// are registered into the table during build_routing_table() via fn pointers.
```

Build-time check: `cargo check -p spendguard-envoy-extproc -p spendguard-egress-proxy` both compile.

## §5. SLICE 2 — Token counter wire

`src/translate/request_phase.rs`:

```rust
use bytes::Bytes;
use serde_json::Value;
use spendguard_provider_routing::{route, resolve_model_id, resolve_tokenizer_kind};
use spendguard_tokenizer::{count_tokens, EncoderKind};

use crate::proto::sidecar::ClaimEstimate;

pub struct ParsedRequest {
    pub path: String,
    pub model: String,
    pub tokenizer_kind: Option<EncoderKind>,
    pub input_tokens: i64,
}

pub fn parse_request_body(path: &str, body: &Bytes) -> anyhow::Result<ParsedRequest> {
    let cfg = route(path).ok_or_else(|| anyhow::anyhow!("unknown_inbound_path: {path}"))?;
    let value: Value = serde_json::from_slice(body)?;

    let model = resolve_model_id(cfg, path, &value);
    let tokenizer_kind = resolve_tokenizer_kind(cfg, path, &value);

    let input_tokens = match tokenizer_kind {
        Some(kind) => count_tokens(kind, &value).unwrap_or(0),
        None => 0,
    };

    Ok(ParsedRequest {
        path: path.to_string(),
        model,
        tokenizer_kind,
        input_tokens,
    })
}

pub fn build_claim_estimate(p: &ParsedRequest) -> ClaimEstimate {
    ClaimEstimate {
        tokenizer_tier: if p.tokenizer_kind.is_some() { "T2".into() } else { "T3".into() },
        tokenizer_version_id: String::new(),  // populated by sidecar in v1.1
        input_tokens: p.input_tokens,
        predicted_a_tokens: estimate_a(p.input_tokens),
        predicted_b_tokens: 0,
        predicted_c_tokens: 0,
        reserved_strategy: "A".into(),
        prediction_strategy_used: "A".into(),
        prediction_policy_used: "STRICT_CEILING".into(),
        prediction_confidence: 0.0,
        prediction_sample_size: 0,
        cold_start_layer_used: String::new(),
        classifier_version: String::new(),
        fingerprint_version: String::new(),
        prompt_class_fingerprint: String::new(),
        run_projection_at_decision_atomic: 0,
        run_predicted_remaining_steps: -1,
        run_steps_completed_so_far: 0,
        run_code_triggered: String::new(),
        model: p.model.clone(),
        prompt_class: String::new(),
    }
}

fn estimate_a(input_tokens: i64) -> i64 {
    // Strategy A: 2× the input as a generous ceiling. Matches the existing
    // egress_proxy SLICE_10 behavior; the sidecar's predictor pipeline will
    // refine if/when the customer wires a plugin.
    input_tokens.saturating_mul(2)
}
```

## §6. SLICE 3 — Budget query path

`src/server.rs` (excerpt):

```rust
use tonic::{Request, Response, Status, Streaming};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::proto::envoy::service::ext_proc::v3::{
    external_processor_server::ExternalProcessor,
    ProcessingRequest, ProcessingResponse,
    processing_request::Request as PReq,
    processing_response::Response as PResp,
    CommonResponse, ImmediateResponse, HttpStatus,
};
use crate::stream::StreamState;
use crate::translate::{request_phase, decision_map};

pub struct ExtProcService {
    sidecar: Arc<crate::sidecar_client::SidecarClient>,
}

#[tonic::async_trait]
impl ExternalProcessor for ExtProcService {
    type ProcessStream = ReceiverStream<Result<ProcessingResponse, Status>>;

    async fn process(
        &self,
        req: Request<Streaming<ProcessingRequest>>,
    ) -> Result<Response<Self::ProcessStream>, Status> {
        let (tx, rx) = mpsc::channel(4);
        let mut input = req.into_inner();
        let sidecar = self.sidecar.clone();

        tokio::spawn(async move {
            let mut state = StreamState::new();
            while let Some(msg) = input.message().await.transpose() {
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => { let _ = tx.send(Err(e)).await; return; }
                };
                let resp = match msg.request {
                    Some(PReq::RequestHeaders(h)) => {
                        state.absorb_request_headers(h);
                        continue_response()
                    }
                    Some(PReq::RequestBody(b)) => {
                        match request_phase::parse_request_body(&state.path(), &b.body.into()) {
                            Ok(parsed) => {
                                let claim = request_phase::build_claim_estimate(&parsed);
                                match sidecar.request_decision(state.session_id(), claim).await {
                                    Ok(decision) => {
                                        state.bind_decision(decision.clone());
                                        decision_map::to_extproc(decision)
                                    }
                                    Err(_) => deny_503(),
                                }
                            }
                            Err(_) => deny_400(),
                        }
                    }
                    Some(PReq::ResponseHeaders(_)) => continue_response(),
                    Some(PReq::ResponseBody(b)) => {
                        // SLICE 4 wires LLM_CALL_POST.SUCCESS here.
                        let _ = state.absorb_response_body(b);
                        continue_response()
                    }
                    _ => continue_response(),
                };
                if tx.send(Ok(resp)).await.is_err() { return; }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

fn continue_response() -> ProcessingResponse { /* ... */ }
fn deny_503() -> ProcessingResponse { /* immediate_response 503 + retry-after */ }
fn deny_400() -> ProcessingResponse { /* immediate_response 400 + body */ }
```

`src/translate/decision_map.rs`:

```rust
use crate::proto::envoy::service::ext_proc::v3::*;
use crate::proto::sidecar::DecisionResponse;
use crate::proto::sidecar::decision_response::Decision;

pub fn to_extproc(d: DecisionResponse) -> ProcessingResponse {
    match Decision::try_from(d.decision).unwrap_or(Decision::Unspecified) {
        Decision::Continue => continue_response(),
        Decision::Stop | Decision::StopRunProjection => deny_429(d.reason_codes),
        Decision::RequireApproval => deny_403_approval(d.approval_request_id),
        Decision::Degrade => mutate_response(d.mutation_patch_json),
        Decision::Skip => continue_response(),
        Decision::Unspecified => deny_503(),
    }
}
```

## §7. SLICE 4 — Audit emit (Response phase)

`src/translate/response_phase.rs` writes `EmitTraceEvents` with a single `LLM_CALL_POST` event carrying the upstream-reported usage extracted via the same `usage_extractor` function the routing table already exposes. The provider-routing crate is re-used; we do NOT re-parse the response body.

```rust
use spendguard_provider_routing::{route, ProviderConfig};

pub fn extract_usage(path: &str, body: &Bytes) -> Option<UsageReport> {
    let cfg = route(path)?;
    let value: Value = serde_json::from_slice(body).ok()?;
    let usage = (cfg.usage_extractor)(&value);
    Some(UsageReport {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total: usage.total_for_commit(),
    })
}
```

Stream lifecycle in `src/stream.rs` holds `reservation_id` + `session_id` + `decision_id` between Request-Body and Response-Body phases. On stream close without a `LLM_CALL_POST.SUCCESS`, we emit `LLM_CALL_POST.RUN_ABORTED` so the sidecar drives the implicit release path (mirrors [`services/sidecar/src/server/adapter_uds.rs:510-573`](../../../../services/sidecar/src/server/adapter_uds.rs)).

## §8. SLICE 5 — Conformance fixtures

`tests/conformance/envoy_v06_token_counting.yaml.json` and `envoy_v06_budget.yaml.json` are golden JSON fixtures taken from the Envoy AI Gateway v0.6 reference manifest examples. The conformance harness:

1. Constructs an in-process `ProcessingRequest` stream identical to what Envoy v0.6 would emit.
2. Runs the `ExtProcService` against a mock sidecar (`tests/conformance/fixtures.rs::MockSidecar`).
3. Asserts the response stream byte-equals the expected golden JSON.

Fixtures live in-tree (vendored); no live network call needed; reviewer re-runs via `cargo test -p spendguard-envoy-extproc --test conformance`.

## §9. SLICE 6 — Helm sub-chart

`charts/spendguard/templates/envoy_extproc.yaml` mirrors `output_predictor.yaml`:

```yaml
{{- if .Values.envoyExtproc.enabled }}
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "spendguard.fullname" . }}-envoy-extproc
  labels: {{- include "spendguard.labels" . | nindent 4 }}
    app.kubernetes.io/component: envoy-extproc
spec:
  replicas: {{ .Values.envoyExtproc.replicaCount }}
  selector:
    matchLabels:
      {{- include "spendguard.selectorLabels" . | nindent 6 }}
      app.kubernetes.io/component: envoy-extproc
  template:
    metadata:
      labels:
        {{- include "spendguard.selectorLabels" . | nindent 8 }}
        app.kubernetes.io/component: envoy-extproc
    spec:
      serviceAccountName: {{ include "spendguard.fullname" . }}-envoy-extproc
      containers:
        - name: envoy-extproc
          image: {{ .Values.envoyExtproc.image.repository }}:{{ .Values.envoyExtproc.image.tag }}
          imagePullPolicy: {{ .Values.envoyExtproc.image.pullPolicy }}
          ports:
            - name: grpc
              containerPort: 8443
            - name: metrics
              containerPort: 9090
          env:
            - name: SPENDGUARD_EXTPROC_BIND_ADDR
              value: "0.0.0.0:8443"
            - name: SPENDGUARD_EXTPROC_SIDECAR_URI
              value: "https://{{ include "spendguard.fullname" . }}-sidecar:8443"
            - name: SPENDGUARD_EXTPROC_TENANT_ID
              value: {{ required "tenant_id required" .Values.tenant_id | quote }}
          volumeMounts:
            - name: svid
              mountPath: /run/secrets/svid
              readOnly: true
          readinessProbe: { httpGet: { path: /readyz, port: 9090 } }
          livenessProbe: { httpGet: { path: /livez, port: 9090 } }
          securityContext:
            runAsNonRoot: true
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities: { drop: ["ALL"] }
      volumes:
        - name: svid
          csi:
            driver: csi.spiffe.io
            readOnly: true
{{- end }}
```

NetworkPolicy entry in `charts/spendguard/templates/networkpolicy.yaml` allows ingress on port 8443 from pods labeled `app.kubernetes.io/name: envoy-ai-gateway` (operator-overridable).

## §10. SLICE 7 — Demo + docs

`deploy/demo/Makefile` adds a new branch:

```make
else ifeq ($(DEMO_MODE),envoy_extproc)
	@echo "[demo] DEMO_MODE=envoy_extproc → postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]                           + envoy-extproc + envoy-proxy + mock-upstream."
	@echo "[demo] Validates the ExtProc translation path end-to-end."
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar envoy-extproc envoy-proxy mock-upstream
```

Verification SQL at `deploy/demo/verify_step_envoy_extproc.sql` asserts:
- ≥ 1 `audit_decision` row with `runtime_kind = 'envoy-ai-gateway'`
- ≥ 1 matching `audit_outcome` row with the same `decision_id`
- `verify-chain` passes

User-facing doc at `docs/site/docs/integrations/envoy-ai-gateway.md` shows the minimal `ExternalProcessor` Envoy config snippet pointing at the new service.

## §11. Configuration surface

| Env var | Default | Purpose |
|---------|---------|---------|
| `SPENDGUARD_EXTPROC_BIND_ADDR` | `0.0.0.0:8443` | gRPC listen address |
| `SPENDGUARD_EXTPROC_SIDECAR_URI` | (required) | `https://host:port` to SpendGuard sidecar mTLS endpoint |
| `SPENDGUARD_EXTPROC_TENANT_ID` | (required) | Tenant assertion forwarded in Handshake |
| `SPENDGUARD_EXTPROC_CA_BUNDLE_PATH` | `/run/secrets/svid/ca.crt` | Sidecar CA trust anchor |
| `SPENDGUARD_EXTPROC_CLIENT_CERT_PATH` | `/run/secrets/svid/tls.crt` | Our SVID client cert |
| `SPENDGUARD_EXTPROC_CLIENT_KEY_PATH` | `/run/secrets/svid/tls.key` | Our SVID key |
| `SPENDGUARD_EXTPROC_FAIL_CLOSED` | `true` | Deny on sidecar unreachable (per §3.4) |
| `SPENDGUARD_EXTPROC_REQUEST_TIMEOUT_MS` | `50` | Hot-path budget for `RequestDecision` |

## §12. Build & CI integration

- `Cargo.toml` workspace `exclude` list extended to include `services/envoy_extproc` (mirroring all other services). The new shared crate `crates/spendguard-provider-routing` is also added to the existing `exclude` list pattern.
- `ci/` (existing GitHub Actions) gets a new step `cargo build -p spendguard-envoy-extproc --release` and `cargo test -p spendguard-envoy-extproc`.
- Container image build follows the same `Dockerfile` pattern as `services/output_predictor/Dockerfile`. Image publish follows the existing chart-image-publish workflow.

## §13. Open implementation questions (resolved)

| Question | Resolution |
|----------|-----------|
| ExtProc v3 vs v3alpha? | v3 — stable in Envoy 1.30+; Envoy AI Gateway v0.6 ships against v3. |
| Per-stream state location? | In-memory map keyed by session_id; lost on restart, sidecar reservation falls to TTL release (matches `services/sidecar/src/server/adapter_uds.rs:385-405` POC pattern). |
| Streaming SSE bodies? | Deferred. v1 commits at end-of-response-body only. Spec §3.5 explicit. |
| Customer plugin C? | Deferred. v1 is Strategy A only. |
