// Tests for the SLICE 3 Access reserve flow. Per review-standards §4
// we exercise: parse-once, DENY → 429+JSON+SPENDGUARD_DENY,
// reservation_id propagation, DEGRADE branching, timeout-as-degrade,
// provider-detection-failure → 400, idempotency-key propagation.

package main

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"sync"
	"testing"
)

// mockKong is a minimal in-process stand-in for kongContext. The
// go-pdk `test` harness can't drive a fake sidecar; this fake lets
// us assert on every PDK side effect.
type mockKong struct {
	mu           sync.Mutex
	body         []byte
	bodyErr      error
	path         string
	pathErr      error
	headers      map[string]string
	headerErr    map[string]error
	shared       map[string]interface{}
	sharedSetErr map[string]error
	exitStatus   int
	exitBody     []byte
	exitHeaders  map[string][]string
	exitCalled   bool
	warnLogs     []string
	errLogs      []string
}

func newMockKong() *mockKong {
	return &mockKong{
		headers:   map[string]string{},
		shared:    map[string]interface{}{},
		headerErr: map[string]error{},
	}
}

func (m *mockKong) GetRawBody() ([]byte, error) { return m.body, m.bodyErr }
func (m *mockKong) GetPath() (string, error)    { return m.path, m.pathErr }
func (m *mockKong) GetHeader(name string) (string, error) {
	if err := m.headerErr[name]; err != nil {
		return "", err
	}
	return m.headers[name], nil
}
func (m *mockKong) SetShared(key string, value interface{}) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if err := m.sharedSetErr[key]; err != nil {
		return err
	}
	m.shared[key] = value
	return nil
}
func (m *mockKong) Exit(status int, body []byte, headers map[string][]string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.exitStatus = status
	m.exitBody = body
	m.exitHeaders = headers
	m.exitCalled = true
}
func (m *mockKong) LogWarn(msg string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.warnLogs = append(m.warnLogs, msg)
}
func (m *mockKong) LogErr(msg string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.errLogs = append(m.errLogs, msg)
}

// mockSidecar implements sidecarTransport. Each test pre-loads the
// response shape it wants to exercise.
type mockSidecar struct {
	tokenizeResp    TokenizeResponse
	tokenizeErr     error
	decisionResp    DecisionResponseBody
	decisionErr     error
	traceResp       TraceAckBody
	traceErr        error
	tokenizeCalls   int
	decisionCalls   int
	traceCalls      int
	lastTokenizeReq TokenizeRequest
	lastDecisionReq DecisionRequestBody
	lastTraceReq    TraceRequestBody
	mu              sync.Mutex
}

func (m *mockSidecar) Tokenize(_ context.Context, req TokenizeRequest) (TokenizeResponse, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.tokenizeCalls++
	m.lastTokenizeReq = req
	return m.tokenizeResp, m.tokenizeErr
}
func (m *mockSidecar) Decision(_ context.Context, req DecisionRequestBody) (DecisionResponseBody, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.decisionCalls++
	m.lastDecisionReq = req
	return m.decisionResp, m.decisionErr
}
func (m *mockSidecar) Trace(_ context.Context, req TraceRequestBody) (TraceAckBody, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.traceCalls++
	m.lastTraceReq = req
	return m.traceResp, m.traceErr
}

// openaiBody is the canonical SLICE 3 happy-path request body.
func openaiBody(t *testing.T) []byte {
	t.Helper()
	b, err := json.Marshal(map[string]interface{}{
		"model": "gpt-4o-mini",
		"messages": []map[string]string{
			{"role": "user", "content": "hello world"},
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func anthropicBody(t *testing.T) []byte {
	t.Helper()
	b, err := json.Marshal(map[string]interface{}{
		"model":      "claude-3-5-sonnet-20241022",
		"max_tokens": 1024,
		"messages": []map[string]string{
			{"role": "user", "content": "ping"},
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// TestAccess_AllowStashesReservationID — review-standards §4.3.
func TestAccess_AllowStashesReservationID(t *testing.T) {
	k := newMockKong()
	k.body = openaiBody(t)
	k.path = "/v1/chat/completions"
	k.headers["Idempotency-Key"] = "client-key-1"

	sc := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 4, TokenizerTier: "T2"},
		decisionResp: DecisionResponseBody{
			Verdict:       "ALLOW",
			ReservationID: "res-uuid-1",
			DecisionID:    "dec-uuid-1",
		},
	}

	cfg := &Config{TenantID: "tenant-1", TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if k.exitCalled {
		t.Fatalf("Exit unexpectedly called: status=%d body=%s", k.exitStatus, k.exitBody)
	}
	if got := k.shared[CtxKeyReservationID]; got != "res-uuid-1" {
		t.Fatalf("reservation_id not stashed: got %v", got)
	}
	if got := k.shared[CtxKeyProvider]; got != string(ProviderOpenAI) {
		t.Fatalf("provider not stashed: got %v", got)
	}
	if sc.lastDecisionReq.IdempotencyKey != "client-key-1" {
		t.Fatalf("idempotency-key not propagated: got %q", sc.lastDecisionReq.IdempotencyKey)
	}
	if sc.lastDecisionReq.ClaimEstimateAtomic != "4" {
		t.Fatalf("claim_estimate_atomic not from tokenizer: got %q", sc.lastDecisionReq.ClaimEstimateAtomic)
	}
}

// TestAccess_DenyReturns429JSONWithSpendguardDeny — review-standards
// §4.2 (DENY MUST be JSON with literal SPENDGUARD_DENY for grep).
func TestAccess_DenyReturns429JSONWithSpendguardDeny(t *testing.T) {
	k := newMockKong()
	k.body = openaiBody(t)
	k.path = "/v1/chat/completions"

	sc := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 4},
		decisionResp: DecisionResponseBody{
			Verdict:     "DENY",
			ReasonCodes: []string{"BUDGET_EXCEEDED"},
		},
	}

	cfg := &Config{TenantID: "t1", TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if !k.exitCalled {
		t.Fatal("Exit not called on DENY")
	}
	if k.exitStatus != 429 {
		t.Fatalf("DENY status: want 429, got %d", k.exitStatus)
	}
	if !strings.Contains(string(k.exitBody), "SPENDGUARD_DENY") {
		t.Fatalf("DENY body missing SPENDGUARD_DENY: %s", k.exitBody)
	}
	ct, ok := k.exitHeaders["Content-Type"]
	if !ok || len(ct) == 0 || ct[0] != "application/json" {
		t.Fatalf("DENY Content-Type missing JSON: %v", k.exitHeaders)
	}
	// Reservation MUST NOT leak on DENY.
	if got := k.shared[CtxKeyReservationID]; got != nil {
		t.Fatalf("reservation_id leaked on DENY: %v", got)
	}
}

// TestAccess_DegradeFailClosedExits503 — §1.6 + §4.4 default closed.
func TestAccess_DegradeFailClosedExits503(t *testing.T) {
	k := newMockKong()
	k.body = openaiBody(t)
	k.path = "/v1/chat/completions"

	sc := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 4},
		decisionResp: DecisionResponseBody{Verdict: "DEGRADE"},
	}

	cfg := &Config{TenantID: "t1", FailOpen: false, TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if !k.exitCalled {
		t.Fatal("DEGRADE with FailOpen=false must exit")
	}
	if k.exitStatus != 503 {
		t.Fatalf("DEGRADE fail-closed status: want 503, got %d", k.exitStatus)
	}
	if !strings.Contains(string(k.exitBody), "SPENDGUARD_DEGRADE") {
		t.Fatalf("DEGRADE body missing SPENDGUARD_DEGRADE: %s", k.exitBody)
	}
}

// TestAccess_DegradeFailOpenContinues — §4.4 operator opt-in.
func TestAccess_DegradeFailOpenContinues(t *testing.T) {
	k := newMockKong()
	k.body = openaiBody(t)
	k.path = "/v1/chat/completions"

	sc := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 4},
		decisionResp: DecisionResponseBody{Verdict: "DEGRADE"},
	}

	cfg := &Config{TenantID: "t1", FailOpen: true, TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if k.exitCalled {
		t.Fatalf("DEGRADE with FailOpen=true must NOT exit; got status=%d", k.exitStatus)
	}
	if got := k.shared[CtxKeyDegraded]; got != "1" {
		t.Fatalf("degraded flag not set: %v", got)
	}
	if len(k.warnLogs) == 0 {
		t.Fatal("DEGRADE with FailOpen=true must emit a warn log")
	}
}

// TestAccess_ProviderDetectionFailureReturns400 — §4.6 (client error
// for unrecognised body shape, NOT server error).
func TestAccess_ProviderDetectionFailureReturns400(t *testing.T) {
	k := newMockKong()
	k.body = []byte(`{"not": "an openai or anthropic body"}`)
	k.path = "/random/path"

	sc := &mockSidecar{}
	cfg := &Config{TenantID: "t1", TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if !k.exitCalled {
		t.Fatal("Exit not called for unrecognised body")
	}
	if k.exitStatus != 400 {
		t.Fatalf("provider-detection failure status: want 400, got %d", k.exitStatus)
	}
	if sc.tokenizeCalls != 0 || sc.decisionCalls != 0 {
		t.Fatal("sidecar must not be called on provider-detection failure")
	}
}

// TestAccess_SidecarTokenizeUnreachableFailsClosed — §4.5 plus §1.6.
func TestAccess_SidecarTokenizeUnreachableFailsClosed(t *testing.T) {
	k := newMockKong()
	k.body = openaiBody(t)
	k.path = "/v1/chat/completions"

	sc := &mockSidecar{
		tokenizeErr: errors.New("dial tcp: connection refused"),
	}
	cfg := &Config{TenantID: "t1", FailOpen: false, TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if !k.exitCalled {
		t.Fatal("tokenize unreachable + FailOpen=false must exit")
	}
	if k.exitStatus != 503 {
		t.Fatalf("tokenize unreachable status: want 503, got %d", k.exitStatus)
	}
	if !strings.Contains(string(k.exitBody), "SPENDGUARD_TOKENIZE_UNREACHABLE") {
		t.Fatalf("tokenize unreachable body missing code: %s", k.exitBody)
	}
}

// TestAccess_SidecarDecisionUnreachableFailOpen — fail-open lets the
// call proceed even when sidecar is unreachable.
func TestAccess_SidecarDecisionUnreachableFailOpen(t *testing.T) {
	k := newMockKong()
	k.body = openaiBody(t)
	k.path = "/v1/chat/completions"

	sc := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 4},
		decisionErr:  errors.New("timeout"),
	}
	cfg := &Config{TenantID: "t1", FailOpen: true, TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if k.exitCalled {
		t.Fatalf("decision unreachable + FailOpen=true must NOT exit; status=%d", k.exitStatus)
	}
	if got := k.shared[CtxKeyDegraded]; got != "1" {
		t.Fatalf("degraded flag not set: %v", got)
	}
}

// TestAccess_AutoIdempotencyKeyWhenHeaderMissing — §4.8 the plugin
// supplies a deterministic key when the upstream client did not.
func TestAccess_AutoIdempotencyKeyWhenHeaderMissing(t *testing.T) {
	k := newMockKong()
	k.body = openaiBody(t)
	k.path = "/v1/chat/completions"

	sc := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 4},
		decisionResp: DecisionResponseBody{Verdict: "ALLOW", ReservationID: "r1"},
	}
	cfg := &Config{TenantID: "t1", TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if sc.lastDecisionReq.IdempotencyKey == "" {
		t.Fatal("auto idempotency-key not generated")
	}
	if !strings.HasPrefix(sc.lastDecisionReq.IdempotencyKey, "kong-auto-") {
		t.Fatalf("auto idempotency-key prefix wrong: %s", sc.lastDecisionReq.IdempotencyKey)
	}

	// Same body → same auto key (deterministic).
	k2 := newMockKong()
	k2.body = openaiBody(t)
	k2.path = "/v1/chat/completions"
	sc2 := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 4},
		decisionResp: DecisionResponseBody{Verdict: "ALLOW", ReservationID: "r2"},
	}
	runAccessWithDeps(k2, cfg, sc2)
	if sc.lastDecisionReq.IdempotencyKey != sc2.lastDecisionReq.IdempotencyKey {
		t.Fatalf("auto idempotency-key not deterministic: %q vs %q",
			sc.lastDecisionReq.IdempotencyKey, sc2.lastDecisionReq.IdempotencyKey)
	}
}

// TestAccess_AnthropicProviderRoute exercises the Anthropic shape.
func TestAccess_AnthropicProviderRoute(t *testing.T) {
	k := newMockKong()
	k.body = anthropicBody(t)
	k.path = "/v1/messages"

	sc := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 2},
		decisionResp: DecisionResponseBody{Verdict: "ALLOW", ReservationID: "r-anth"},
	}
	cfg := &Config{TenantID: "t1", TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if k.exitCalled {
		t.Fatalf("ALLOW must not exit; status=%d", k.exitStatus)
	}
	if got := k.shared[CtxKeyProvider]; got != string(ProviderAnthropic) {
		t.Fatalf("anthropic provider not stashed: %v", got)
	}
	if sc.lastTokenizeReq.Provider != string(ProviderAnthropic) {
		t.Fatalf("tokenize provider wrong: %s", sc.lastTokenizeReq.Provider)
	}
	if sc.lastDecisionReq.ModelClass != "anthropic/claude-3-5-sonnet-20241022" {
		t.Fatalf("model_class wrong: %s", sc.lastDecisionReq.ModelClass)
	}
}

// TestAccess_IdempotencyConflictReturns409 — §4.8 conflict passthrough.
func TestAccess_IdempotencyConflictReturns409(t *testing.T) {
	k := newMockKong()
	k.body = openaiBody(t)
	k.path = "/v1/chat/completions"

	sc := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 4},
		decisionErr:  &SidecarError{Status: 409, Code: "SPENDGUARD_IDEMPOTENCY_CONFLICT", Message: "dup"},
	}
	cfg := &Config{TenantID: "t1", FailOpen: true, TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if !k.exitCalled {
		t.Fatal("409 must short-circuit even with FailOpen=true")
	}
	if k.exitStatus != 409 {
		t.Fatalf("idempotency conflict status: want 409, got %d", k.exitStatus)
	}
}

// TestAccess_AllowWithEmptyReservationIDFailsClosed — §4.3
// invariant that ALLOW carries a reservation; otherwise companion
// is mis-wired and we MUST fail closed.
func TestAccess_AllowWithEmptyReservationIDFailsClosed(t *testing.T) {
	k := newMockKong()
	k.body = openaiBody(t)
	k.path = "/v1/chat/completions"

	sc := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 4},
		decisionResp: DecisionResponseBody{Verdict: "ALLOW", ReservationID: ""},
	}
	cfg := &Config{TenantID: "t1", FailOpen: false, TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if !k.exitCalled {
		t.Fatal("ALLOW without reservation_id must fail closed")
	}
	if k.exitStatus != 503 {
		t.Fatalf("ALLOW-no-reservation status: want 503, got %d", k.exitStatus)
	}
	if !strings.Contains(string(k.exitBody), "SPENDGUARD_RESERVATION_MISSING") {
		t.Fatalf("ALLOW-no-reservation body code missing: %s", k.exitBody)
	}
}

// TestAccess_DefaultTimeoutUsedWhenConfigZero — review-standards
// §3.4 (sensible default for TimeoutMS).
func TestAccess_DefaultTimeoutUsedWhenConfigZero(t *testing.T) {
	k := newMockKong()
	k.body = openaiBody(t)
	k.path = "/v1/chat/completions"

	sc := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 4},
		decisionResp: DecisionResponseBody{Verdict: "ALLOW", ReservationID: "r"},
	}
	cfg := &Config{TenantID: "t1", TimeoutMS: 0}
	runAccessWithDeps(k, cfg, sc)
	// Test passes if no panic / timeout-related Exit fired. The
	// 500ms default keeps the unit test safely under the run
	// budget; mockSidecar returns synchronously so the deadline
	// never actually fires.
	if k.exitCalled {
		t.Fatalf("default timeout cfg should not exit on happy path; status=%d", k.exitStatus)
	}
}

// TestAccess_UnknownVerdictFailsClosed — defense in depth: an
// unrecognised verdict from the sidecar means our wire is out of
// sync; fail-closed.
func TestAccess_UnknownVerdictFailsClosed(t *testing.T) {
	k := newMockKong()
	k.body = openaiBody(t)
	k.path = "/v1/chat/completions"

	sc := &mockSidecar{
		tokenizeResp: TokenizeResponse{InputTokens: 4},
		decisionResp: DecisionResponseBody{Verdict: "MAYBE"},
	}
	cfg := &Config{TenantID: "t1", FailOpen: false, TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if !k.exitCalled {
		t.Fatal("unknown verdict must fail closed")
	}
	if k.exitStatus != 503 {
		t.Fatalf("unknown verdict status: want 503, got %d", k.exitStatus)
	}
	if !strings.Contains(string(k.exitBody), "SPENDGUARD_UNKNOWN_VERDICT") {
		t.Fatalf("unknown-verdict code missing: %s", k.exitBody)
	}
}

// TestAccess_BodyReadFailureFailsClosed — §4.1 plus §1.6.
func TestAccess_BodyReadFailureFailsClosed(t *testing.T) {
	k := newMockKong()
	k.bodyErr = errors.New("kong body buffer drained")
	k.path = "/v1/chat/completions"

	sc := &mockSidecar{}
	cfg := &Config{TenantID: "t1", FailOpen: false, TimeoutMS: 500}
	runAccessWithDeps(k, cfg, sc)

	if !k.exitCalled {
		t.Fatal("body read failure must exit when FailOpen=false")
	}
	if k.exitStatus != 502 {
		t.Fatalf("body read failure status: want 502, got %d", k.exitStatus)
	}
	if sc.tokenizeCalls != 0 {
		t.Fatal("sidecar must not be called when body read fails")
	}
}
