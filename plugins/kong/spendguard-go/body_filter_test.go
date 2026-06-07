// Tests for the SLICE 4 BodyFilter commit flow. Per review-standards
// §5 we exercise: chunk accumulation, end-of-body single-trace
// invariant (§5.1), plugin-side dedup flag (§5.2), missing
// reservation_id silent skip (§5.3), provider parse failure →
// REJECTED (§5.4-5.5), upstream 5xx → REJECTED, commit-timeout
// non-exit (§5.6), OpenAI + Anthropic usage parsing.

package main

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"sync"
	"testing"
)

// mockBodyKong is the BodyFilter analogue of mockKong.
type mockBodyKong struct {
	mu             sync.Mutex
	shared         map[string]string
	sharedRaw      map[string]interface{}
	sharedSetErr   map[string]error
	chunkQueue     [][]byte
	chunkErr       error
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
func (m *mockBodyKong) GetSharedAny(key string) (interface{}, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if v, ok := m.sharedRaw[key]; ok {
		return v, nil
	}
	if v, ok := m.shared[key]; ok {
		return v, nil
	}
	return nil, nil
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
func (m *mockBodyKong) GetChunk() ([]byte, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.chunkErr != nil {
		return nil, m.chunkErr
	}
	if len(m.chunkQueue) == 0 {
		return []byte{}, nil
	}
	c := m.chunkQueue[0]
	m.chunkQueue = m.chunkQueue[1:]
	return c, nil
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

// TestBodyFilter_ChunkedAccumulationFiresOnceOnEOB — §5.1 single
// trace at end-of-body across multiple chunks.
func TestBodyFilter_ChunkedAccumulationFiresOnceOnEOB(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "res-1"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.upstreamStatus = 200

	full := openaiResponseBody(t)
	split := len(full) / 2
	k.chunkQueue = [][]byte{full[:split], full[split:], {}}

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "ACCEPTED", LedgerTransactionID: "lt-1"}}
	cfg := &Config{TimeoutMS: 500}

	// First chunk — buffer.
	runBodyFilterWithDeps(k, cfg, sc)
	if sc.traceCalls != 0 {
		t.Fatal("trace fired on partial body")
	}
	// Second chunk — buffer.
	runBodyFilterWithDeps(k, cfg, sc)
	if sc.traceCalls != 0 {
		t.Fatal("trace fired on second partial chunk")
	}
	// Final empty chunk — should fire trace exactly once.
	runBodyFilterWithDeps(k, cfg, sc)
	if sc.traceCalls != 1 {
		t.Fatalf("trace must fire exactly once on EOB: got %d", sc.traceCalls)
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
	k.chunkQueue = [][]byte{openaiResponseBody(t), {}}

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "ACCEPTED"}}
	cfg := &Config{TimeoutMS: 500}

	// Fire end-to-end.
	runBodyFilterWithDeps(k, cfg, sc) // buffer
	runBodyFilterWithDeps(k, cfg, sc) // EOB → commit
	if sc.traceCalls != 1 {
		t.Fatalf("trace not fired on EOB: %d calls", sc.traceCalls)
	}
	// Simulate Kong re-entry (committed flag is set).
	k.chunkQueue = [][]byte{{}}
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
	k.chunkQueue = [][]byte{openaiResponseBody(t), {}}

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "ACCEPTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc) // buffer
	runBodyFilterWithDeps(k, cfg, sc) // EOB

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
	k.chunkQueue = [][]byte{anthropicResponseBody(t), {}}

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "ACCEPTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc) // buffer
	runBodyFilterWithDeps(k, cfg, sc) // EOB

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
	k.chunkQueue = [][]byte{[]byte("<html>502 Bad Gateway</html>"), {}}

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "REJECTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc) // buffer
	runBodyFilterWithDeps(k, cfg, sc) // EOB

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
	k.chunkQueue = [][]byte{[]byte("not json"), {}}

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "REJECTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc) // buffer
	runBodyFilterWithDeps(k, cfg, sc) // EOB

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
	k.chunkQueue = [][]byte{[]byte(`{"id":"x","usage":{"input":5}}`), {}}

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "REJECTED"}}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc) // buffer
	runBodyFilterWithDeps(k, cfg, sc) // EOB

	if sc.traceCalls != 1 {
		t.Fatalf("unknown provider trace not fired: %d calls", sc.traceCalls)
	}
	if sc.lastTraceReq.Outcome != "REJECTED" {
		t.Fatalf("unknown provider outcome: want REJECTED, got %s", sc.lastTraceReq.Outcome)
	}
}

// TestBodyFilter_CommitTimeoutDoesNotShortCircuit — §5.6 sidecar
// failure on the commit lane MUST NOT exit the request (response
// already in flight). The trace warn is logged.
func TestBodyFilter_CommitTimeoutDoesNotShortCircuit(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "r-tot"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.chunkQueue = [][]byte{openaiResponseBody(t), {}}

	sc := &mockSidecar{
		traceErr: errors.New("context deadline exceeded"),
	}
	cfg := &Config{TimeoutMS: 500}
	runBodyFilterWithDeps(k, cfg, sc) // buffer
	runBodyFilterWithDeps(k, cfg, sc) // EOB

	// Trace was attempted exactly once.
	if sc.traceCalls != 1 {
		t.Fatalf("trace must attempt once on commit lane: %d calls", sc.traceCalls)
	}
	// Failure logged but no panic. Plugin-side dedup still set.
	if k.shared[CtxKeyCommitted] != "1" {
		t.Fatal("committed flag not set after timeout")
	}
}

// TestBodyFilter_SetSharedFailureLogsButDoesNotPanic — defensive
// path: if SetShared fails during buffering, log and continue.
func TestBodyFilter_SetSharedFailureLogsButDoesNotPanic(t *testing.T) {
	k := newMockBodyKong()
	k.shared[CtxKeyReservationID] = "r-share-err"
	k.shared[CtxKeyProvider] = string(ProviderOpenAI)
	k.sharedSetErr = map[string]error{
		CtxKeyBodyBuffer: errors.New("ctx.shared write rejected"),
	}
	k.chunkQueue = [][]byte{[]byte(`{"partial":true}`), {}}

	sc := &mockSidecar{traceResp: TraceAckBody{Verdict: "REJECTED"}}
	cfg := &Config{TimeoutMS: 500}
	// First call: SetShared(buffer) fails; we log+return.
	runBodyFilterWithDeps(k, cfg, sc)
	if sc.traceCalls != 0 {
		t.Fatal("trace fired despite SetShared failure mid-buffer")
	}
	// Second call: empty chunk fires the trace on the now-empty
	// (because SetShared failed) buffer. parseProviderUsage fails
	// (empty body) and emits REJECTED, which is correct fail-closed
	// commit semantics.
	runBodyFilterWithDeps(k, cfg, sc)
	if sc.traceCalls != 1 || sc.lastTraceReq.Outcome != "REJECTED" {
		t.Fatalf("expected one REJECTED trace, got calls=%d outcome=%s", sc.traceCalls, sc.lastTraceReq.Outcome)
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

// TestIsFinalChunk_BoundaryConditions — Kong terminator semantics.
func TestIsFinalChunk_BoundaryConditions(t *testing.T) {
	if !isFinalChunk([]byte{}) {
		t.Fatal("empty must be final")
	}
	if isFinalChunk([]byte("a")) {
		t.Fatal("non-empty must not be final")
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
