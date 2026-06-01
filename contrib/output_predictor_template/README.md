# SpendGuard Output Predictor Plugin — Reference Template

Reference Python implementation of a **Strategy C** output predictor
plugin for [SpendGuard](https://github.com/m24927605/agentic-spendguard).

> **STUB MODEL — REPLACE BEFORE PRODUCTION.**
>
> The `model_predictor_stub.py` shipped here is a 10-row toy
> `LinearRegression` whose only job is to exercise the gRPC wire
> surface, the Dockerfile, and the conformance corpus. **Do not run it
> against real budgets.** The first thing you should do in your fork
> is replace `StubModel` with your trained predictor.

Spec ancestor: `docs/output-predictor-plugin-contract-v1alpha1.md`
§10 (template surface) — read it before touching this code.

License: **Apache 2.0** (matches SpendGuard main repository).

---

## What this template provides

- `predictor_server.py` — gRPC server implementing
  `CustomerPredictor.Predict` + `HealthCheck` from
  `proto/spendguard/output_predictor_plugin/v1/plugin.proto`.
- `feature_extractor.py` — deterministic conversion from
  `PredictRequest` to a fixed-shape numpy feature vector.
- `model_predictor_stub.py` — **stub** sklearn `LinearRegression`
  trained on 10 toy rows. **Replace me.**
- `backtest_harness.py` — offline calibration check against historical
  audit data; emits P50/P95/P99 of `actual / predicted` ratios.
- `conformance_test.py` — pytest suite exercising 8 failure modes
  per `output-predictor-plugin-contract-v1alpha1.md` §5.1 and the 50-
  request corpus.
- `Dockerfile` — Python 3.11-slim, multi-stage, non-root UID 65532,
  EXPOSE 50054, `grpc_health_probe` baked in.
- `mtls_setup.md` — end-to-end mTLS cert provisioning walkthrough
  (cert-manager + control plane registration).
- `gen_proto.sh` — regenerates Python bindings from the proto.

## 5-minute quickstart

```bash
# 1. Set up a venv and install runtime deps.
python3.11 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt

# 2. Regenerate proto bindings (skip if `_proto/` already exists).
pip install "grpcio-tools>=1.62,<2"
bash gen_proto.sh

# 3. Run the server INSECURE for local development.
python predictor_server.py --insecure --port 50054

# 4. In another terminal, run the conformance suite.
pip install pytest
python -m pytest conformance_test.py -v
```

You should see ~10+ tests pass. The server logs every Predict
invocation including request id and elapsed microseconds.

## Container quickstart

```bash
# Build (no platform pinning — adjust --platform on M-series hosts).
docker build -t spendguard-plugin-template:dev .

# Run without mTLS, port-forward 50054.
docker run --rm -p 50054:50054 spendguard-plugin-template:dev

# Probe health.
docker run --network host fullstorydev/grpcurl \
  -plaintext localhost:50054 \
  grpc.health.v1.Health/Check
```

For production mTLS-enabled deployment, see `mtls_setup.md`.

## Replacing the stub model

1. Train your model on your audit data. The CSV schema expected by
   `backtest_harness.py` is documented at the top of that file.
2. Persist the trained model however you like (`joblib`, `onnx`,
   PyTorch state dict, …). Load it in `__init__` of your
   replacement class.
3. Implement the same surface as `StubModel`:

   ```python
   class MyModel:
       MODEL_VERSION = "my-bigger-tree-v17"

       def __init__(self):
           self._model = joblib.load("artifacts/predictor.joblib")
           self._sample_size = 100_000

       def predict(self, X):
           return np.rint(self._model.predict(X)).astype(np.int64).clip(1)

       def confidence(self, X):
           # Return calibrated confidence in [0, 1] per row.
           return self._model.predict_proba(X).max(axis=1)

       @property
       def sample_size(self):
           return self._sample_size

       def predict_one(self, x):
           ...  # same shape as StubModel.predict_one
   ```

4. Run the backtest harness against held-out audit data:

   ```bash
   python backtest_harness.py --csv data/my_holdout.csv --model-module my_model
   ```

   Iterate on training until the overall P95 calibration ratio stays
   within `[0.95, 1.05]`. The harness exits with code 3 if it falls
   outside that band so CI can gate retraining merges.

5. Bump `MODEL_VERSION` every retrain. SpendGuard stamps this into the
   audit row via `PredictResponse.plugin_version`, which lets you
   correlate downstream drift with specific retrains.

## Why a stub at all?

Per `output-predictor-plugin-contract-v1alpha1.md` §1.1, SpendGuard
deliberately does **not** ship a hosted ML predictor — your ML team
knows your agents better, and a hosted model would create cross-tenant
leak risk. So the contract is a wire surface; the model body is yours
to build. The stub exists so you can stand up the wire surface and
the conformance corpus *before* you have a trained model.

## Conformance corpus

`conformance_test.py` exercises:

- the 8 failure modes from spec §5.1 (timeout, gRPC error,
  zero/negative output, overflow, illegal confidence, deserialization
  error, TLS failure, `NOT_SERVING` health),
- a 50-request happy-path corpus across the 7 prompt classes and
  major model families,
- the tenant-isolation contract (§7.2): a request with a different
  `tenant_id` than the configured one is rejected.

Run it with `python -m pytest conformance_test.py -v`. The suite
should be 100% green on the stub before you replace the model; it
also serves as a regression harness for your replacement model.

## Certification path

Customer production onboarding is documented in:

- `../../docs/customer/plugin-onboarding.md`
- `../../docs/customer/plugin-certification-checklist.md`
- `../../docs/customer/plugin-error-taxonomy.md`

Before SpendGuard Strategy C traffic is enabled for a tenant, produce
the certification evidence in that checklist. At minimum:

```bash
python3 -m pytest conformance_test.py -q
```

The production deployment must use mTLS, must reject plaintext traffic,
and must validate the exact SpendGuard predictor-client SVID URI SAN:

```text
spiffe://spendguard.platform/predictor-client/<tenant_id>
```

The plugin should be idempotent by `spendguard_call_id`, should not
perform unbounded retries inside the 50 ms `Predict` budget, and should
expect SpendGuard's circuit breaker to fall back to Strategy B on
timeouts, gRPC errors, invalid predictions, TLS errors, or
`NOT_SERVING` health.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| `UNAVAILABLE` on the first call | Server still starting; HEALTHCHECK has a 20s `start-period`. |
| `UNAUTHENTICATED` | Client cert not chained to `PREDICTOR_TLS_CLIENT_CA`. See `mtls_setup.md` §2. |
| `INVALID_ARGUMENT: tenant_id does not match` | `PREDICTOR_TENANT_ID` env var disagrees with SpendGuard's stored value. Update one. |
| Backtest exits with code 3 | P95 calibration ratio drifted outside `[0.95, 1.05]`. Retrain. |
| `ImportError: _proto.spendguard...` | `gen_proto.sh` not run, or you copied the source into a venv without it. Re-run the script. |

## Versioning

This template tracks `v1alpha1` of the plugin contract. SpendGuard
commits to additive evolution (proto3) and a 12-month deprecation
notice on any breaking change (per spec §0.4 + §11.2). Pin the
template's git SHA in your fork so you control when to upgrade.

## License

```
Apache License 2.0 — see ../../LICENSE.
```

Files in `_proto/` are generated from `.proto` files in the SpendGuard
monorepo and inherit the same Apache 2.0 license.
