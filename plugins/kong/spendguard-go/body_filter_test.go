// Tests for the SLICE 4 BodyFilter commit flow. Per review-standards
// §5 we exercise: single-shot full-body read + exactly-once trace
// (§5.1), plugin-side dedup flag (§5.2), missing reservation_id silent
// skip (§5.3), provider parse failure → REJECTED (§5.4-5.5), upstream
// 5xx → REJECTED, commit-lane bounded retry + non-exit (§5.6), and the
// real-PDK RawBodyResult contract (resolveRawBody) including the
// oversized-body temp-file branch.

package main

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/Kong/go-pdk/server/kong_plugin_protocol"
)

// mockBodyKong is the BodyFilter analogue of mockKong. It models the
// REAL Kong PDK contract: `kong.service.response.get_raw_body` returns
// the entire buffered body in a single call (see GetFullBody), not a
// chunk stream.
type mockBodyKong struct {
	mu             sync.Mutex
	shared         map[string]string
	sharedRaw      map[string]interface{}
	sharedSetErr   map[string]error
	body           []byte
	bodyErr        error
	upstreamStatus int
	statusErr      error
	warnLogs       []string
	errLogs        []string
}

func newMockBodyKong() *mockBodyKong {
	return &mockBodyKong{
		shared:    map[string]string{},
		sharedRaw: map[string]interface{}{},
	}
}

func (m *mockBodyKong) GetSharedString(key string) (string, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if v, ok := m.shared[key]; ok {
		return v, nil
	}
	return "", nil
}
func (m *mockBodyKong) SetShared(key string, value interface{}) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if err := m.sharedSetErr[key]; err != nil {
		return err
	}
	if s, ok := value.(string); ok {
		m.shared[key] = s
	}
	m.sharedRaw[key] = value
	return nil
}

// GetFullBody returns the full buffered body in one shot, matching the
// real PDK `get_raw_body` semantics. The chunk-stream model used by an
// earlier (broken) version of this mock never fired the commit on real
// Kong because get_raw_body never returns an empty-chunk terminator.
func (m *mockBodyKong) GetFullBody() ([]byte, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.bodyErr != nil {
		return nil, m.bodyErr
	}
	return m.body, nil
}
func (m *mockBodyKong) GetUpstreamStatus() (int, error) { return m.upstreamStatus, m.statusErr }
func (m *mockBodyKong) LogWarn(msg string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.warnLogs = append(m.warnLogs, msg)
}
func (m *mockBodyKong) LogErr(msg string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.errLogs = append(m.errLogs, msg)
}

func openaiResponseBody(t *testing.T) []byte {
	t.Helper()
	b, err := json.Marshal(map[string]interface{}{
		"id": "chatcmpl-test-1",
		"usage": map[string]int{
			"prompt_tokens":     8,
			"completion_tokens": 16,
			"total_tokens":      24,
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func anthropicResponseBody(t *testing.T) []byte {
	t.Helper()
	b, err := json.Marshal(map[string]interface{}{
		"id": "msg_test_1",
		"usage": map[string]int{
			"input_tokens":  5,
			"output_tokens": 10,
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// TestBodyFilter_NoReservationSkipsSilently — §5.3 (access phase
// did not reserve; body_filter must not error or trace).
func TestBodyFilter_NoReservationSkipsSilently(t *testing.T) {
	k := newMockBodyKong()
	// No reservation_id stashed.
	sc := &mockSidecar{}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc)

	if sc.traceCalls != 0 {
		t.Fatalf("trace called without reservation_id: %d calls", sc.traceCalls)
	}
	if len(k.errLogs) != 0 {
		t.Fatalf("no-reservation path must not log errors: %v", k.errLogs)
	}
}

// TestBodyFilter_SingleShotFiresOnceOnFullBody — §5.1: the full
// buffered body arrives in one get_raw_body call and the trace fires
// exactly once. Re-invoking body_filter (Kong teardown replay) must
// NOT fire a second trace — the dedup flag short-circuits it. This is
// the regression guard for the original bug where the code waited for
// an empty-chunk terminator that real Kong never sends, so the commit
// never fired at all.
func TestBodyFilter_SingleShotFiresOnceOnFullBody(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "res-1"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.upstreamStatus = 200
	k.body = openaiResponseBody(t)

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "ACCEPTED", LedgerTransactionID: "lt-1"}}
	cfg := &Config{TimeoutMS: 500}

	// Single body_filter invocation with the full buffered body —
	// must commit immediately, no waiting for a non-existent
	// terminator.
	runBodyFilterWithDeps(k, cfg, sc)
	if sc.traceCalls != 1 {
		t.Fatalf("trace must fire exactly once on the full-body read: got %d", sc.traceCalls)
	}
	// Kong re-invokes body_filter on teardown — dedup flag must
	// short-circuit so the count stays at exactly 1.
	runBodyFilterWithDeps(k, cfg, sc)
	if sc.traceCalls != 1 {
		t.Fatalf("re-entry must dedup; want 1 trace call, got %d", sc.traceCalls)
	}
	if sc.lastTraceReq.Outcome != "ACCEPTED" {
		t.Fatalf("trace outcome: want ACCEPTED, got %s", sc.lastTraceReq.Outcome)
	}
	if sc.lastTraceReq.InputTokens == nil || *sc.lastTraceReq.InputTokens != 8 {
		t.Fatalf("trace input_tokens: want 8, got %v", sc.lastTraceReq.InputTokens)
	}
	if sc.lastTraceReq.OutputTokens == nil || *sc.lastTraceReq.OutputTokens != 16 {
		t.Fatalf("trace output_tokens: want 16, got %v", sc.lastTraceReq.OutputTokens)
	}
}

// TestBodyFilter_PluginSideDedupFlag — §5.2 (a second invocation
// after commit MUST short-circuit even if Kong replays).
func TestBodyFilter_PluginSideDedupFlag(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "res-2"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.body = openaiResponseBody(t)

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "ACCEPTED"}}
	cfg := &Config{TimeoutMS: 500}

	// Fire end-to-end on the first invocation.
	runBodyFilterWithDeps(k, cfg, sc)
	if sc.traceCalls != 1 {
		t.Fatalf("trace not fired on full-body read: %d calls", sc.traceCalls)
	}
	// Simulate Kong re-entry (committed flag is set).
	runBodyFilterWithDeps(k, cfg, sc)
	if sc.traceCalls != 1 {
		t.Fatalf("dedup failed; want 1 trace call, got %d", sc.traceCalls)
	}
}

// TestBodyFilter_OpenAIUsageParsing covers the §5.4 happy path for
// OpenAI body shape.
func TestBodyFilter_OpenAIUsageParsing(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "r-oai"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.body = openaiResponseBody(t)

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "ACCEPTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc)

	if sc.lastTraceReq.ProviderEventID == nil || *sc.lastTraceReq.ProviderEventID != "chatcmpl-test-1" {
		t.Fatalf("openai response id not propagated: %v", sc.lastTraceReq.ProviderEventID)
	}
	if sc.lastTraceReq.ActualAmountAtomic == nil || *sc.lastTraceReq.ActualAmountAtomic != "24" {
		t.Fatalf("actual_amount_atomic wrong: %v", sc.lastTraceReq.ActualAmountAtomic)
	}
}

// TestBodyFilter_AnthropicUsageParsing covers §5.4 for Anthropic.
func TestBodyFilter_AnthropicUsageParsing(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "r-anth"
	k.shared[CtxKeyProvider] = string(ProviderAnthropic)
	k.body = anthropicResponseBody(t)

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "ACCEPTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc)

	if sc.lastTraceReq.InputTokens == nil || *sc.lastTraceReq.InputTokens != 5 {
		t.Fatalf("anthropic input_tokens: want 5, got %v", sc.lastTraceReq.InputTokens)
	}
	if sc.lastTraceReq.OutputTokens == nil || *sc.lastTraceReq.OutputTokens != 10 {
		t.Fatalf("anthropic output_tokens: want 10, got %v", sc.lastTraceReq.OutputTokens)
	}
	if sc.lastTraceReq.ProviderEventID == nil || *sc.lastTraceReq.ProviderEventID != "msg_test_1" {
		t.Fatalf("anthropic id not propagated: %v", sc.lastTraceReq.ProviderEventID)
	}
}

// TestBodyFilter_Upstream5xxEmitsRejected — §5.4 upstream error
// path; reservation gets released, not committed.
func TestBodyFilter_Upstream5xxEmitsRejected(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "r-5xx"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.upstreamStatus = 502
	// Some 5xx bodies are HTML; the parser shouldn't even be
	// called because of the early upstream-status check.
	k.body = []byte("<html>502 Bad Gateway</html>")

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "REJECTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc)

	if sc.traceCalls != 1 {
		t.Fatalf("5xx trace not fired: %d calls", sc.traceCalls)
	}
	if sc.lastTraceReq.Outcome != "REJECTED" {
		t.Fatalf("5xx outcome: want REJECTED, got %s", sc.lastTraceReq.Outcome)
	}
}

// TestBodyFilter_MalformedJSONEmitsRejected — §5.5 parse failure
// fall-through.
func TestBodyFilter_MalformedJSONEmitsRejected(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "r-bad"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.body = []byte("not json")

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "REJECTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc)

	if sc.traceCalls != 1 {
		t.Fatalf("malformed body trace not fired: %d calls", sc.traceCalls)
	}
	if sc.lastTraceReq.Outcome != "REJECTED" {
		t.Fatalf("malformed body outcome: want REJECTED, got %s", sc.lastTraceReq.Outcome)
	}
	if !strings.Contains(strings.Join(k.warnLogs, " "), "parse usage") {
		t.Fatalf("parse-fail warning missing; logs=%v", k.warnLogs)
	}
}

// TestBodyFilter_UnknownProviderEmitsRejected — §5.4 unknown
// provider must NOT silently commit.
func TestBodyFilter_UnknownProviderEmitsRejected(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "r-unknown"
	k.shared[CtxKeyProvider] = "cohere"
	k.body = []byte(`{"id":"x","usage":{"input":5}}`)

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "REJECTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc)

	if sc.traceCalls != 1 {
		t.Fatalf("unknown provider trace not fired: %d calls", sc.traceCalls)
	}
	if sc.lastTraceReq.Outcome != "REJECTED" {
		t.Fatalf("unknown provider outcome: want REJECTED, got %s", sc.lastTraceReq.Outcome)
	}
}

// TestBodyFilter_CommitTimeoutDoesNotShortCircuit — §5.6 sidecar
// failure on the commit lane MUST NOT exit the request (response
// already in flight). The trace warn is logged. With a persistently
// failing sidecar the bounded retry exhausts its attempts but still
// does not panic/exit, and the dedup flag stays set so re-entry won't
// re-attempt.
func TestBodyFilter_CommitTimeoutDoesNotShortCircuit(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "r-tot"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.body = openaiResponseBody(t)

	sc := &mockSidecar{
		traceErr: errors.New("context deadline exceeded"),
	}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc)

	// Bounded retry: every attempt fails, so we exhaust the cap.
	if sc.traceCalls != commitMaxAttempts {
		t.Fatalf("commit lane must retry up to the cap: want %d calls, got %d",
			commitMaxAttempts, sc.traceCalls)
	}
	// Failure logged but no panic. Plugin-side dedup still set.
	if k.shared[CtxKeyCommitted] != "1" {
		t.Fatal("committed flag not set after timeout")
	}
	// Re-entry must NOT re-attempt — dedup short-circuits.
	runBodyFilterWithDeps(k, cfg, sc)
	if sc.traceCalls != commitMaxAttempts {
		t.Fatalf("re-entry re-attempted commit; want %d, got %d", commitMaxAttempts, sc.traceCalls)
	}
}

// TestBodyFilter_CommitRetriesThenSucceeds — the bounded commit-lane
// retry recovers a transient sidecar blip so a realized-spend commit
// is NOT silently dropped to TTL-sweep. The first attempt fails, the
// second succeeds; the commit is idempotent on reservation_id so the
// retry is safe against double-commit.
func TestBodyFilter_CommitRetriesThenSucceeds(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "r-retry"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.body = openaiResponseBody(t)

	sc := &mockSidecar{
		traceResp:          TraceAckBody{Verdict: "ACCEPTED", LedgerTransactionID: "lt-retry"},
		traceTransientErr:  errors.New("connection reset"),
		traceErrsRemaining: 1, // fail once (transient), then succeed
	}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc)

	if sc.traceCalls != 2 {
		t.Fatalf("commit must retry once then succeed: want 2 calls, got %d", sc.traceCalls)
	}
	if sc.lastTraceReq.Outcome != "ACCEPTED" {
		t.Fatalf("recovered commit outcome: want ACCEPTED, got %s", sc.lastTraceReq.Outcome)
	}
	// No leftover warn for a recovered commit.
	if strings.Contains(strings.Join(k.warnLogs, " "), "trace failed") {
		t.Fatalf("recovered commit must not log a failure: %v", k.warnLogs)
	}
}

// TestBodyFilter_BodyReadErrorEmitsRejected — defensive path: if
// get_raw_body errors we cannot know realized usage, so we RELEASE the
// reservation (REJECTED) rather than leaving it to TTL-sweep, and we
// log the failure. Fail-closed: a hold is never left permanently open.
func TestBodyFilter_BodyReadErrorEmitsRejected(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "r-body-err"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.bodyErr = errors.New("get_raw_body failed")

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "REJECTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc)

	if sc.traceCalls != 1 || sc.lastTraceReq.Outcome != "REJECTED" {
		t.Fatalf("expected one REJECTED trace, got calls=%d outcome=%s", sc.traceCalls, sc.lastTraceReq.Outcome)
	}
	if !strings.Contains(strings.Join(k.warnLogs, " "), "get body") {
		t.Fatalf("body-read failure warning missing; logs=%v", k.warnLogs)
	}
}

// TestBodyFilter_EmptyBodyEmitsRejected — an empty buffered body (no
// usage to commit) must RELEASE, not silently commit zeros.
func TestBodyFilter_EmptyBodyEmitsRejected(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "r-empty"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.upstreamStatus = 200
	k.body = nil

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "REJECTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc)

	if sc.traceCalls != 1 || sc.lastTraceReq.Outcome != "REJECTED" {
		t.Fatalf("empty body must RELEASE: calls=%d outcome=%s", sc.traceCalls, sc.lastTraceReq.Outcome)
	}
}

// TestParseOpenAIUsage_DirectUnit covers the helper directly so
// future refactors keep the contract.
func TestParseOpenAIUsage_DirectUnit(t *testing.T) {
	u, err := parseOpenAIUsage(openaiResponseBody(t))
	if err != nil {
		t.Fatal(err)
	}
	if u.InputTokens != 8 || u.OutputTokens != 16 || u.ResponseID != "chatcmpl-test-1" {
		t.Fatalf("parseOpenAIUsage: %+v", u)
	}
}

// TestParseAnthropicUsage_DirectUnit covers Anthropic parsing
// directly. Guards review-standards §5.4 contract.
func TestParseAnthropicUsage_DirectUnit(t *testing.T) {
	u, err := parseAnthropicUsage(anthropicResponseBody(t))
	if err != nil {
		t.Fatal(err)
	}
	if u.InputTokens != 5 || u.OutputTokens != 10 || u.ResponseID != "msg_test_1" {
		t.Fatalf("parseAnthropicUsage: %+v", u)
	}
}

// TestParseProviderUsage_EmptyBodyRejected matches §5.5.
func TestParseProviderUsage_EmptyBodyRejected(t *testing.T) {
	_, err := parseProviderUsage(string(ProviderOpenAI), nil)
	if err == nil {
		t.Fatal("empty body must fail parse")
	}
}

// TestResolveRawBody_RealPDKContract drives the actual go-pdk
// `RawBodyResult` protobuf — the same type
// `kong.service.response.get_raw_body` populates on a live Kong —
// through the adapter's body resolver. This replaces the old synthetic
// chunk-queue mock that did not match the real PDK contract (the real
// API returns the WHOLE body in one call, optionally via a temp-file
// path for oversized bodies). It proves all three oneof branches the
// PDK can send are handled, including the temp-file branch the stock
// `Response.GetRawBody()` helper silently drops.
func TestResolveRawBody_RealPDKContract(t *testing.T) {
	t.Run("inline_content_full_body", func(t *testing.T) {
		want := []byte(`{"usage":{"prompt_tokens":3,"completion_tokens":4}}`)
		out := &kong_plugin_protocol.RawBodyResult{
			Kind: &kong_plugin_protocol.RawBodyResult_Content{Content: want},
		}
		got, err := resolveRawBody(out)
		if err != nil {
			t.Fatal(err)
		}
		if string(got) != string(want) {
			t.Fatalf("inline content mismatch: got %q", got)
		}
	})

	t.Run("oversized_body_temp_file", func(t *testing.T) {
		// Kong spills bodies larger than the Nginx buffer to a temp
		// file and returns the path; we must read it back rather than
		// treat the response as empty.
		dir := t.TempDir()
		path := filepath.Join(dir, "resp.json")
		want := []byte(`{"usage":{"input_tokens":11,"output_tokens":22}}`)
		if err := os.WriteFile(path, want, 0o600); err != nil {
			t.Fatal(err)
		}
		out := &kong_plugin_protocol.RawBodyResult{
			Kind: &kong_plugin_protocol.RawBodyResult_BodyFilepath{BodyFilepath: path},
		}
		got, err := resolveRawBody(out)
		if err != nil {
			t.Fatal(err)
		}
		if string(got) != string(want) {
			t.Fatalf("temp-file body mismatch: got %q", got)
		}
	})

	t.Run("error_branch", func(t *testing.T) {
		out := &kong_plugin_protocol.RawBodyResult{
			Kind: &kong_plugin_protocol.RawBodyResult_Error{Error: "buffer too small"},
		}
		_, err := resolveRawBody(out)
		if err == nil || !strings.Contains(err.Error(), "buffer too small") {
			t.Fatalf("error branch must surface the PDK error: %v", err)
		}
	})
}

// TestResolveRawBody_TempFileFeedsUsageParse closes the loop: a body
// that arrives via the temp-file branch is parsed for usage exactly
// like an inline body, so an oversized but valid response still
// commits realized spend (not a spurious RELEASE).
func TestResolveRawBody_TempFileFeedsUsageParse(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "resp.json")
	if err := os.WriteFile(path, openaiResponseBody(t), 0o600); err != nil {
		t.Fatal(err)
	}
	out := &kong_plugin_protocol.RawBodyResult{
		Kind: &kong_plugin_protocol.RawBodyResult_BodyFilepath{BodyFilepath: path},
	}
	body, err := resolveRawBody(out)
	if err != nil {
		t.Fatal(err)
	}
	usage, err := parseProviderUsage(string(ProviderOpenAI), body)
	if err != nil {
		t.Fatalf("oversized valid body must parse: %v", err)
	}
	if usage.InputTokens != 8 || usage.OutputTokens != 16 {
		t.Fatalf("temp-file usage parse wrong: %+v", usage)
	}
}

// Compile-time assertion that mockSidecar satisfies sidecarTransport.
// Prevents drift if either side adds a method.
var _ sidecarTransport = (*mockSidecar)(nil)

// Compile-time assertion that mockBodyKong satisfies kongBodyContext.
var _ kongBodyContext = (*mockBodyKong)(nil)

// Likewise for mockKong + kongContext (access side).
var _ kongContext = (*mockKong)(nil)

// silence unused — context import retained for future tests.
var _ = context.TODO
