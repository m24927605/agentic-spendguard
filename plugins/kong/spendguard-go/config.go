// Package main — Kong plugin configuration struct.
//
// Per `docs/specs/coverage/D09_kong_ai_gateway/implementation.md` §2
// and `review-standards.md` §1.6 + §3.4 the configuration is
// fail-closed by default: `FailOpen=false`, and `TimeoutMS` defaults to
// a non-zero value (500ms) so an unconfigured plugin still has a
// reasonable upper bound on sidecar latency. SLICE 2 ships the struct
// + defaulting constructor only; SLICE 3 wires it into Access, SLICE 4
// into BodyFilter, SLICE 6 into Helm.
//
// Distribution: this file compiles to a Kong go-plugin-server `.so`
// alongside main.go via `make build-kong-plugin`. The `Config` struct
// shape is the wire contract with `KongPlugin` CRDs and `kong.conf`
// stanzas; do not rename JSON tags without a coordinated chart bump.

package main

// Config holds all per-plugin (or per-route) parameters Kong supplies
// at every `Access` / `BodyFilter` invocation. Field tags drive the
// JSON shape the Kong control plane / declarative `kong.yml` writes;
// keep them stable.
type Config struct {
	// SidecarURL — full HTTPS URL of the SpendGuard sidecar HTTP
	// companion. Default empty; SLICE 6 wires `https://spendguard-kong-companion.<ns>.svc.cluster.local:8443`
	// from the Helm chart. Per design §3.1 this is mTLS-only; the
	// plugin refuses to dial plaintext URLs in SLICE 3.
	SidecarURL string `json:"sidecar_url"`

	// SidecarCAPEM — inline PEM bundle of the CA chain that signs the
	// sidecar's workload cert. Used by the Go `crypto/tls` client
	// builder in SLICE 3. Either this or `SidecarCAFile` must be
	// non-empty in SLICE 3 onward; SLICE 2 ships the struct only.
	SidecarCAPEM string `json:"sidecar_ca_pem"`

	// SidecarCAFile — path to a CA bundle on disk. Mounted by the
	// Helm chart from a Kubernetes Secret. Alternative to
	// SidecarCAPEM; the two are mutually exclusive (validation
	// happens in SLICE 3's `loadTLSConfig`).
	SidecarCAFile string `json:"sidecar_ca_file"`

	// ClientCertPEM — workload cert presented to the sidecar.
	// SVID-style URI SAN encodes the tenant; HARDEN_08 / SLICE 6
	// wires per-tenant SVIDs from cert-manager.
	ClientCertPEM string `json:"client_cert_pem"`
	ClientCertFile string `json:"client_cert_file"`

	// ClientKeyPEM — private key matching the workload cert.
	ClientKeyPEM string `json:"client_key_pem"`
	ClientKeyFile string `json:"client_key_file"`

	// TenantID — tenant assertion sent to the sidecar in every
	// /v1/decision call. Must match the SVID URI SAN tenant; SLICE 3
	// enforces. Empty TenantID + non-empty SidecarURL is a startup
	// error (fail-closed per review-standards §1.6).
	TenantID string `json:"tenant_id"`

	// FailOpen — per design §3.4. When false (default), sidecar
	// errors return `kong.response.exit(503)` upstream of the LLM
	// call. When true, the plugin logs the degradation and lets the
	// upstream call proceed unprotected. Operators MUST set this
	// explicitly in `KongPlugin` CRDs; the plugin emits a startup
	// log warning when FailOpen=true so the deviation is auditable.
	FailOpen bool `json:"fail_open"`

	// TimeoutMS — per-request timeout for the sidecar HTTP client.
	// Default 500ms keeps the plugin inside Kong's typical p99
	// upstream budget. SLICE 3 treats a timeout as DEGRADE, not
	// hard error (review-standards §4.5).
	TimeoutMS int `json:"timeout_ms"`

	// PluginVersion — surfaced to Kong's plugin-server registry +
	// reflected in /metrics labels (SLICE 6). Hardcoded here so
	// version skew between the binary and the CRD is detectable.
	PluginVersion string `json:"-"`
}

// New constructs a Config with the documented defaults. Kong's
// go-plugin-server passes the result through every plugin invocation
// before merging in operator-supplied values from `KongPlugin` /
// `kong.conf`; review-standards §3.4 + §1.6 require these defaults
// stay fail-closed.
func New() interface{} {
	return &Config{
		TimeoutMS:     defaultTimeoutMS,
		FailOpen:      false,
		PluginVersion: PluginVersion,
	}
}

// defaultTimeoutMS is the per-request HTTP timeout the plugin uses
// when the operator does not override `TimeoutMS`. Kept as a constant
// so the unit test can assert on it without re-instantiating.
const defaultTimeoutMS = 500
