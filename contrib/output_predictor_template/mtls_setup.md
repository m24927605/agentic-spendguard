# mTLS setup for the SpendGuard output predictor plugin

This walkthrough produces a working mTLS chain so SpendGuard can call
your customer-hosted Strategy C plugin. Follow it once per tenant.

Spec refs: `output-predictor-plugin-contract-v1alpha1.md` §3 (mTLS auth)
and §7 (per-tenant isolation). SLICE_07 (plugin contract) enforces these
on the SpendGuard side; the template enforces them client-side.

## 0. What you need

- A tenant UUIDv7 that SpendGuard's control plane has provisioned for
  your organization (call it `${TENANT_ID}`).
- A way to operate certificates in your cluster. The example below uses
  [cert-manager](https://cert-manager.io); if you already run Vault PKI
  / AWS Private CA / step-ca, substitute accordingly.
- `kubectl` access to the namespace where the plugin will run.

## 1. Mint the plugin's TLS server identity

The plugin's server cert is **yours**: SpendGuard does not issue it.
Your CA signs it; you publish the public-key fingerprint to SpendGuard
so SpendGuard pins it (per spec §3.1).

### cert-manager example

```yaml
# plugin-server-cert.yaml
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: predictor-plugin-server
  namespace: predictor
spec:
  secretName: predictor-plugin-server-tls
  duration: 720h          # 30 days; rotate well within
  renewBefore: 240h
  subject:
    organizations: ["acme.example"]
  commonName: "predictor.acme.example"
  dnsNames:
    - predictor.acme.example
    - predictor.predictor.svc.cluster.local
  usages:
    - server auth
    - digital signature
    - key encipherment
  issuerRef:
    name: acme-internal-ca
    kind: ClusterIssuer
```

Apply, then inspect:

```
kubectl apply -f plugin-server-cert.yaml
kubectl -n predictor get secret predictor-plugin-server-tls -o yaml
```

The Secret contains `tls.crt` (server cert PEM) and `tls.key`
(private key). Mount these into the plugin container as
`PREDICTOR_TLS_SERVER_CERT` and `PREDICTOR_TLS_SERVER_KEY`.

### Compute the server cert fingerprint

SpendGuard's control plane stores the SHA-256 fingerprint of your
server cert (per spec §3.1) and refuses to connect if it changes
without a rotation event. Generate it:

```
kubectl -n predictor get secret predictor-plugin-server-tls -o jsonpath='{.data.tls\.crt}' \
  | base64 -d \
  | openssl x509 -fingerprint -sha256 -noout \
  | awk -F= '{print $2}' | tr -d ':' | tr '[:upper:]' '[:lower:]'
```

Record the resulting 64-character hex string; you upload it in §3.

## 2. Trust SpendGuard's client-cert CA and SVID subject

SpendGuard issues a per-tenant client SVID with subject
`spiffe://spendguard.platform/predictor-client/${TENANT_ID}` (spec §3.1).
Your plugin must verify both the client cert chain and that URI SAN exactly
matches the `tenant_id` in `PredictRequest`. The template does this when
`PREDICTOR_TLS_CLIENT_CA` is set; `--require-client-svid` is available for
tests and local mTLS smoke runs.

Get SpendGuard's CA bundle from the control plane:

```
curl --fail \
     --header "Authorization: Bearer ${SPENDGUARD_API_TOKEN}" \
     https://control-plane.spendguard.example/api/v1/trust/predictor-client-ca \
     > spendguard-predictor-client-ca.pem
```

Drop it into your cluster as a ConfigMap or Secret:

```
kubectl -n predictor create configmap predictor-trust-bundle \
  --from-file=spendguard-ca.pem=spendguard-predictor-client-ca.pem
```

Mount it into the plugin container as `PREDICTOR_TLS_CLIENT_CA`.

> **Refresh cadence**: SpendGuard rotates the predictor-client CA
> alongside the 30-day cert rotation in spec §3.2. Re-fetch
> `predictor-client-ca` monthly via cron, or subscribe to the control
> plane's `spendguard.plugin.ca_rotated` CloudEvent webhook.

## 3. Register the endpoint with SpendGuard

Once your plugin is reachable behind a stable URL, register it via the
SpendGuard control plane (per spec §8):

```
curl --fail -X POST \
     --header "Authorization: Bearer ${SPENDGUARD_API_TOKEN}" \
     --header "Content-Type: application/json" \
     --data @- \
     https://control-plane.spendguard.example/api/v1/predictor-plugins <<EOF
{
  "tenant_id": "${TENANT_ID}",
  "endpoint_url": "https://predictor.acme.example:50054",
  "server_cert_fingerprint": "${FINGERPRINT_FROM_§1}"
}
EOF
```

The response includes `plugin_endpoint_id` and `client_cert_chain_pem`.
The chain is informational — SpendGuard's runtime presents the
matching client cert automatically; the PEM is for your audit trail.

Per spec §8 each registration emits a signed
`spendguard.plugin.registered` CloudEvent. Verify it lands in your
audit feed before declaring the integration complete.

## 4. Launch the plugin

In your Helm chart / Deployment manifest, point the container at the
mounted certs:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: predictor-plugin
  namespace: predictor
spec:
  replicas: 2          # high-availability; circuit breaker is per-pod
  selector:
    matchLabels:
      app: predictor-plugin
  template:
    metadata:
      labels:
        app: predictor-plugin
    spec:
      containers:
      - name: predictor
        image: ghcr.io/acme/spendguard-predictor:latest
        ports:
        - containerPort: 50054
          name: grpc
        env:
        - name: PREDICTOR_TENANT_ID
          value: "${TENANT_ID}"
        - name: PREDICTOR_TLS_SERVER_CERT
          value: /certs/server/tls.crt
        - name: PREDICTOR_TLS_SERVER_KEY
          value: /certs/server/tls.key
        - name: PREDICTOR_TLS_CLIENT_CA
          value: /trust/spendguard-ca.pem
        - name: PREDICTOR_REQUIRE_CLIENT_SVID
          value: "true"
        args: []        # no --insecure in production
        volumeMounts:
        - name: server-cert
          mountPath: /certs/server
          readOnly: true
        - name: trust-bundle
          mountPath: /trust
          readOnly: true
      volumes:
      - name: server-cert
        secret:
          secretName: predictor-plugin-server-tls
      - name: trust-bundle
        configMap:
          name: predictor-trust-bundle
```

## 5. Verify the round trip

From a host that holds the SpendGuard-issued tenant client cert mounted as
`tls.crt` / `tls.key`, verify the cert subject first:

```
openssl x509 -in tls.crt -noout -ext subjectAltName \
  | grep "spiffe://spendguard.platform/predictor-client/${TENANT_ID}"
```

Then call HealthCheck:

```
grpc_health_probe \
  -addr=predictor.acme.example:50054 \
  -tls \
  -tls-ca-cert=spendguard-ca.pem \
  -tls-client-cert=client.pem \
  -tls-client-key=client.key \
  -service=spendguard.output_predictor_plugin.v1.CustomerPredictor
```

You should see `status: SERVING`. If you see `UNAVAILABLE` or
`UNAUTHENTICATED`, work through the checklist in `README.md` →
"Troubleshooting".

## 6. Rotate without downtime

When cert-manager renews the server cert (spec §3.2 dual-validity
window is 12 hours):

1. Cert-manager writes the new cert into the same Secret.
2. The plugin container picks it up on the next gRPC TLS handshake
   (the template loads creds once at startup; `kubectl rollout restart`
   the Deployment to refresh — or run two replicas so SpendGuard never
   loses all paths during the restart).
3. Compute the new SHA-256 fingerprint (`§1`) and `PUT` it on the
   control plane endpoint (`/api/v1/predictor-plugins/{id}`).
4. SpendGuard accepts the new fingerprint immediately while keeping
   the old one valid for the dual-validity window.

The control plane emits a signed `spendguard.plugin.updated` event;
log it for compliance.
