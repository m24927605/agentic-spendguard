// Tests for sidecar_client.go. We don't bring up real mTLS here —
// that's covered end-to-end against the live sidecar in
// `slice3_4_e2e_test.go` (build-tag gated). These tests cover the
// pure pieces: config validation, PEM loading from disk, traversal
// rejection, and the JSON wire helpers via a plain httptest.Server.

package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestNewSidecarClient_RejectsEmptyURL(t *testing.T) {
	_, err := newSidecarClient(&Config{})
	if err == nil {
		t.Fatal("empty sidecar_url must error")
	}
}

func TestNewSidecarClient_RejectsPlaintext(t *testing.T) {
	_, err := newSidecarClient(&Config{SidecarURL: "http://localhost:8443"})
	if err == nil || !strings.Contains(err.Error(), "HTTPS") {
		t.Fatalf("plaintext url must error: %v", err)
	}
}

func TestNewSidecarClient_RejectsNilConfig(t *testing.T) {
	_, err := newSidecarClient(nil)
	if err == nil {
		t.Fatal("nil cfg must error")
	}
}

func TestLoadPEM_RejectsPathTraversal(t *testing.T) {
	_, err := loadPEM("", "../../etc/passwd", "client_cert")
	if err == nil || !strings.Contains(err.Error(), "traversal") {
		t.Fatalf("path traversal not rejected: %v", err)
	}
}

func TestLoadPEM_RejectsMissingBoth(t *testing.T) {
	_, err := loadPEM("", "", "client_cert")
	if err == nil {
		t.Fatal("missing both pem + path must error")
	}
}

func TestLoadPEM_ReadsFromDisk(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "cert.pem")
	if err := os.WriteFile(path, []byte("dummy pem"), 0o600); err != nil {
		t.Fatal(err)
	}
	data, err := loadPEM("", path, "client_cert")
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "dummy pem" {
		t.Fatalf("read mismatch: %s", data)
	}
}

// httpStubClient builds a SidecarClient whose Transport points at a
// plain httptest.Server (we bypass TLS so we can assert on the JSON
// wire shape without rcgen). The mTLS path is covered by the live
// sidecar test harness.
type httpStubClient struct {
	*SidecarClient
}

func newHTTPStub(t *testing.T, handler http.HandlerFunc) (*httpStubClient, *httptest.Server) {
	t.Helper()
	srv := httptest.NewServer(handler)
	c := &SidecarClient{
		cfg: &Config{TimeoutMS: 500},
		http: &http.Client{
			Timeout: srv.Client().Timeout,
		},
		baseURL: srv.URL,
	}
	return &httpStubClient{SidecarClient: c}, srv
}

func TestSidecarClient_TokenizeJSONWireShape(t *testing.T) {
	c, srv := newHTTPStub(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/tokenize" {
			t.Errorf("path: %s", r.URL.Path)
		}
		if r.Header.Get("Content-Type") != "application/json" {
			t.Errorf("content-type: %s", r.Header.Get("Content-Type"))
		}
		var req TokenizeRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			t.Errorf("decode: %v", err)
		}
		if req.Provider != "openai" || req.Model != "gpt-4o-mini" {
			t.Errorf("payload mismatch: %+v", req)
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(TokenizeResponse{InputTokens: 7, TokenizerTier: "T2"})
	})
	defer srv.Close()

	resp, err := c.Tokenize(context.Background(), TokenizeRequest{
		Provider: "openai", Model: "gpt-4o-mini", Prompt: "hi",
	})
	if err != nil {
		t.Fatal(err)
	}
	if resp.InputTokens != 7 {
		t.Fatalf("input_tokens: %d", resp.InputTokens)
	}
}

func TestSidecarClient_DecisionAllowResponse(t *testing.T) {
	c, srv := newHTTPStub(t, func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(DecisionResponseBody{
			Verdict: "ALLOW", ReservationID: "r1", DecisionID: "d1",
		})
	})
	defer srv.Close()

	resp, err := c.Decision(context.Background(), DecisionRequestBody{
		TenantID: "t1", ClaimEstimateAtomic: "100", IdempotencyKey: "k1",
	})
	if err != nil {
		t.Fatal(err)
	}
	if resp.Verdict != "ALLOW" || resp.ReservationID != "r1" {
		t.Fatalf("decision response wrong: %+v", resp)
	}
}

func TestSidecarClient_Non2xxReturnsSidecarError(t *testing.T) {
	c, srv := newHTTPStub(t, func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(503)
		_ = json.NewEncoder(w).Encode(map[string]string{
			"error": "ledger down",
			"code":  "SPENDGUARD_DEPENDENCY_UNAVAILABLE",
		})
	})
	defer srv.Close()

	_, err := c.Decision(context.Background(), DecisionRequestBody{
		TenantID: "t1", ClaimEstimateAtomic: "100", IdempotencyKey: "k",
	})
	if err == nil {
		t.Fatal("503 must produce error")
	}
	sErr, ok := err.(*SidecarError)
	if !ok {
		t.Fatalf("expected *SidecarError, got %T: %v", err, err)
	}
	if sErr.Status != 503 || sErr.Code != "SPENDGUARD_DEPENDENCY_UNAVAILABLE" {
		t.Fatalf("SidecarError fields wrong: %+v", sErr)
	}
}
