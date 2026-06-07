// Package main — sidecar HTTP+mTLS client (D09 SLICE 3).
//
// Per `docs/specs/coverage/D09_kong_ai_gateway/design.md` §3.1 the
// plugin talks to the sidecar over HTTPS+mTLS only. This file owns:
//
//  1. tls.Config assembly from the operator-supplied PEMs.
//  2. The `*http.Client` (one per Config, reused across requests so
//     review-standards §10.3 mTLS handshake re-use holds).
//  3. JSON encode/decode for /v1/tokenize, /v1/decision, /v1/trace.
//
// The wire shapes mirror `services/sidecar/src/http_companion/
// handlers.rs` 1:1. Stability is enforced by the integration tests
// that drive a live sidecar; see `slice3_4_e2e_test.go`.
//
// Anti-scope:
//
//   - No protobuf in this layer. The sidecar's HTTP companion is
//     JSON-over-HTTP/1.1; protobuf lives behind the in-process gRPC
//     adapter UDS and is unreachable from Kong's workers.
//   - No connection-pool tuning beyond Go's defaults; SLICE 6
//     surfaces tuning knobs in the Helm chart.

package main

import (
	"bytes"
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// SidecarClient is the per-config HTTP client. Build once via
// `newSidecarClient(cfg)`, reuse across `Access` / `BodyFilter`
// invocations within the lifetime of the plugin process.
type SidecarClient struct {
	cfg     *Config
	http    *http.Client
	baseURL string
}

// Wire shapes (mirrors services/sidecar/src/http_companion/handlers.rs).
//
//revive:disable:exported wire-stability documented above
type TokenizeRequest struct {
	Provider string `json:"provider"`
	Model    string `json:"model"`
	Prompt   string `json:"prompt"`
}

type TokenizeResponse struct {
	InputTokens        uint32 `json:"input_tokens"`
	TokenizerTier      string `json:"tokenizer_tier"`
	TokenizerVersionID string `json:"tokenizer_version_id"`
}

type DecisionRequestBody struct {
	TenantID            string `json:"tenant_id"`
	ClaimEstimateAtomic string `json:"claim_estimate_atomic"`
	PromptClass         string `json:"prompt_class"`
	ModelClass          string `json:"model_class"`
	IdempotencyKey      string `json:"idempotency_key"`
	BudgetID            string `json:"budget_id,omitempty"`
}

type DecisionResponseBody struct {
	Verdict       string   `json:"verdict"` // "ALLOW" | "DENY" | "DEGRADE"
	ReservationID string   `json:"reservation_id"`
	DecisionID    string   `json:"decision_id"`
	ReasonCodes   []string `json:"reason_codes"`
}

type TraceRequestBody struct {
	ReservationID      string  `json:"reservation_id"`
	Outcome            string  `json:"outcome"` // "ACCEPTED" | "REJECTED"
	ProviderEventID    *string `json:"provider_event_id,omitempty"`
	InputTokens        *uint32 `json:"input_tokens,omitempty"`
	OutputTokens       *uint32 `json:"output_tokens,omitempty"`
	ActualAmountAtomic *string `json:"actual_amount_atomic,omitempty"`
}

type TraceAckBody struct {
	Verdict             string `json:"verdict"`
	LedgerTransactionID string `json:"ledger_transaction_id"`
}

// SidecarError is returned for any non-2xx response. Keeps the HTTP
// status code so the Access flow can decide DENY vs DEGRADE.
type SidecarError struct {
	Status  int
	Code    string
	Message string
}

func (e *SidecarError) Error() string {
	return fmt.Sprintf("spendguard sidecar: %d %s (%s)", e.Status, e.Code, e.Message)
}

// newSidecarClient assembles the per-config HTTP client. Errors here
// are caller-fatal — review-standards §1.6 fail-closed default means
// a misconfigured plugin MUST refuse to load.
func newSidecarClient(cfg *Config) (*SidecarClient, error) {
	if cfg == nil {
		return nil, errors.New("spendguard: nil config")
	}
	if cfg.SidecarURL == "" {
		return nil, errors.New("spendguard: sidecar_url required")
	}
	if !strings.HasPrefix(cfg.SidecarURL, "https://") {
		// Per design §3.1 mTLS is mandatory; refuse plaintext URLs
		// up front so the failure mode is loud at configuration
		// time rather than silent on the first call.
		return nil, fmt.Errorf("spendguard: sidecar_url must be HTTPS, got %q", cfg.SidecarURL)
	}
	tlsCfg, err := loadTLSConfig(cfg)
	if err != nil {
		return nil, fmt.Errorf("spendguard: tls: %w", err)
	}

	timeout := time.Duration(cfg.TimeoutMS) * time.Millisecond
	if timeout <= 0 {
		timeout = defaultTimeoutMS * time.Millisecond
	}
	client := &http.Client{
		Timeout: timeout,
		Transport: &http.Transport{
			TLSClientConfig: tlsCfg,
			// Re-use connections so mTLS handshakes amortize
			// (review-standards §10.3). Defaults pool 100
			// idle conns / host which is plenty for a single
			// Kong worker.
			MaxIdleConns:        100,
			MaxIdleConnsPerHost: 100,
			IdleConnTimeout:     90 * time.Second,
		},
	}
	return &SidecarClient{
		cfg:     cfg,
		http:    client,
		baseURL: strings.TrimRight(cfg.SidecarURL, "/"),
	}, nil
}

// loadTLSConfig honours the config's PEM-or-path choice. Per
// review-standards §9.7 the path variants are stat'd via
// filepath.Clean to defeat trivial traversal; production wiring
// reads them from the Helm chart Secret mount under
// `/var/run/secrets/spendguard`.
func loadTLSConfig(cfg *Config) (*tls.Config, error) {
	caBundle, err := loadPEM(cfg.SidecarCAPEM, cfg.SidecarCAFile, "sidecar_ca")
	if err != nil {
		return nil, err
	}
	pool := x509.NewCertPool()
	if !pool.AppendCertsFromPEM(caBundle) {
		return nil, errors.New("spendguard: sidecar_ca PEM had no parseable certs")
	}

	certPEM, err := loadPEM(cfg.ClientCertPEM, cfg.ClientCertFile, "client_cert")
	if err != nil {
		return nil, err
	}
	keyPEM, err := loadPEM(cfg.ClientKeyPEM, cfg.ClientKeyFile, "client_key")
	if err != nil {
		return nil, err
	}
	clientCert, err := tls.X509KeyPair(certPEM, keyPEM)
	if err != nil {
		return nil, fmt.Errorf("spendguard: client cert/key pair: %w", err)
	}

	return &tls.Config{
		RootCAs:      pool,
		Certificates: []tls.Certificate{clientCert},
		MinVersion:   tls.VersionTLS13,
	}, nil
}

// loadPEM returns inline PEM bytes if non-empty, else reads from
// disk. Path traversal defense: filepath.Clean + reject `..`
// segments (review-standards §9.7).
func loadPEM(inline, path, name string) ([]byte, error) {
	if inline != "" {
		return []byte(inline), nil
	}
	if path == "" {
		return nil, fmt.Errorf("spendguard: %s_pem or %s_file required", name, name)
	}
	clean := filepath.Clean(path)
	if strings.Contains(clean, "..") {
		return nil, fmt.Errorf("spendguard: %s path %q rejected (traversal)", name, path)
	}
	data, err := os.ReadFile(clean)
	if err != nil {
		return nil, fmt.Errorf("spendguard: read %s: %w", name, err)
	}
	return data, nil
}

// Tokenize POSTs /v1/tokenize. Returns SidecarError on non-2xx so
// callers can translate to DENY/DEGRADE per review-standards §4.5.
func (c *SidecarClient) Tokenize(ctx context.Context, req TokenizeRequest) (TokenizeResponse, error) {
	var resp TokenizeResponse
	err := c.postJSON(ctx, "/v1/tokenize", req, &resp)
	return resp, err
}

// Decision POSTs /v1/decision. The reservation_id is empty on DENY
// by sidecar contract; the caller MUST NOT propagate empty IDs into
// `kong.ctx.shared`.
func (c *SidecarClient) Decision(ctx context.Context, req DecisionRequestBody) (DecisionResponseBody, error) {
	var resp DecisionResponseBody
	err := c.postJSON(ctx, "/v1/decision", req, &resp)
	return resp, err
}

// Trace POSTs /v1/trace. Per review-standards §5.2 the caller is
// responsible for the plugin-side dedup flag; the sidecar handles
// ledger-side idempotency on reservation_id.
func (c *SidecarClient) Trace(ctx context.Context, req TraceRequestBody) (TraceAckBody, error) {
	var resp TraceAckBody
	err := c.postJSON(ctx, "/v1/trace", req, &resp)
	return resp, err
}

// postJSON is the single hot-path helper. Keeps payload encoding +
// status-code translation in one place.
func (c *SidecarClient) postJSON(ctx context.Context, endpoint string, payload, out interface{}) error {
	body, err := json.Marshal(payload)
	if err != nil {
		return fmt.Errorf("encode %s body: %w", endpoint, err)
	}
	httpReq, err := http.NewRequestWithContext(ctx, "POST", c.baseURL+endpoint, bytes.NewReader(body))
	if err != nil {
		return fmt.Errorf("build %s request: %w", endpoint, err)
	}
	httpReq.Header.Set("Content-Type", "application/json")
	httpReq.Header.Set("User-Agent", "spendguard-kong/"+PluginVersion)

	httpResp, err := c.http.Do(httpReq)
	if err != nil {
		return fmt.Errorf("post %s: %w", endpoint, err)
	}
	defer httpResp.Body.Close()

	bodyBytes, err := io.ReadAll(io.LimitReader(httpResp.Body, 4*1024*1024))
	if err != nil {
		return fmt.Errorf("read %s response: %w", endpoint, err)
	}

	if httpResp.StatusCode >= 200 && httpResp.StatusCode < 300 {
		if out == nil {
			return nil
		}
		if err := json.Unmarshal(bodyBytes, out); err != nil {
			return fmt.Errorf("decode %s response: %w", endpoint, err)
		}
		return nil
	}

	// Non-2xx: try to parse the sidecar's WireError shape
	// `{"error": "...", "code": "SPENDGUARD_..."}`. If decoding
	// fails we still surface the status code so callers can
	// branch.
	var wire struct {
		Error string `json:"error"`
		Code  string `json:"code"`
	}
	_ = json.Unmarshal(bodyBytes, &wire)
	return &SidecarError{
		Status:  httpResp.StatusCode,
		Code:    wire.Code,
		Message: wire.Error,
	}
}
