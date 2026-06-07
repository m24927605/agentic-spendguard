// Package main — Kong BodyFilter phase commit flow (D09 SLICE 4).
//
// Per `docs/specs/coverage/D09_kong_ai_gateway/design.md` §3.3:
//
//  1. Kong streams the upstream response body in chunks; we
//     accumulate them in `kong.ctx.shared` until the final empty
//     chunk arrives.
//  2. On end-of-body we parse the provider usage:
//     * OpenAI: `{"usage":{"prompt_tokens","completion_tokens"}}`
//     * Anthropic: `{"usage":{"input_tokens","output_tokens"}}`
//  3. POST /v1/trace with `LLM_CALL_POST.SUCCESS` (mapped to
//     ACCEPTED on the HTTP wire) carrying:
//     * the reservation_id stashed by `access.go`,
//     * the provider-reported token counts,
//     * the upstream response id for provider-side dedup.
//  4. Upstream 5xx → POST /v1/trace with REJECTED so the
//     reservation is released.
//
// Idempotency: the sidecar handles ledger-side dedup on
// reservation_id (review-standards §5.2). The plugin-side dedup
// flag stops a second Trace call inside the same Kong request when
// body_filter fires more than once on the same logical end-of-body.
//
// Per review-standards §5.6 a sidecar timeout on the commit lane is
// logged but does not exit the request — the upstream response is
// already on its way back to the client. Test
// `body_filter_commit_timeout_does_not_short_circuit` exercises that.

package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/Kong/go-pdk"
)

// CtxKeyBodyBuffer accumulates the streamed response chunks.
const CtxKeyBodyBuffer = "spendguard_body_buffer"

// CtxKeyCommitted is the plugin-side dedup flag from
// review-standards §5.2 — once we've fired Trace for this request,
// subsequent body_filter invocations short-circuit silently.
const CtxKeyCommitted = "spendguard_committed"

// CtxKeyUpstreamStatus is set by the body filter when we observe a
// non-2xx upstream status so the commit/release branch sees it
// after end-of-body.
const CtxKeyUpstreamStatus = "spendguard_upstream_status"

// kongBodyContext is the BodyFilter analogue of kongContext. We
// only need 3 PDK touchpoints + log.
type kongBodyContext interface {
	GetSharedString(key string) (string, error)
	SetShared(key string, value interface{}) error
	GetSharedAny(key string) (interface{}, error)
	GetChunk() ([]byte, error)
	GetUpstreamStatus() (int, error)
	LogWarn(msg string)
	LogErr(msg string)
}

type pdkBodyAdapter struct{ k *pdk.PDK }

func (a *pdkBodyAdapter) GetSharedString(k string) (string, error) {
	return a.k.Ctx.GetSharedString(k)
}
func (a *pdkBodyAdapter) SetShared(k string, v interface{}) error { return a.k.Ctx.SetShared(k, v) }
func (a *pdkBodyAdapter) GetSharedAny(k string) (interface{}, error) {
	return a.k.Ctx.GetSharedAny(k)
}
func (a *pdkBodyAdapter) GetChunk() ([]byte, error) {
	return a.k.ServiceResponse.GetRawBody()
}
func (a *pdkBodyAdapter) GetUpstreamStatus() (int, error) {
	return a.k.ServiceResponse.GetStatus()
}
func (a *pdkBodyAdapter) LogWarn(m string) { _ = a.k.Log.Warn(m) }
func (a *pdkBodyAdapter) LogErr(m string)  { _ = a.k.Log.Err(m) }

// runBodyFilter is the production entry point invoked by
// `main.BodyFilter`. SLICE 4 lazily reads the reservation; if the
// access phase short-circuited (DENY) there is no reservation and
// we exit silently (review-standards §5.3).
func runBodyFilter(k *pdk.PDK, cfg *Config) {
	client, err := newSidecarClient(cfg)
	if err != nil {
		// Cannot fail-close in body_filter — the upstream
		// response is already in flight (review-standards §5.6).
		_ = k.Log.Err("spendguard body_filter: client init: " + err.Error())
		return
	}
	runBodyFilterWithDeps(&pdkBodyAdapter{k: k}, cfg, client)
}

// runBodyFilterWithDeps is the unit-testable core.
func runBodyFilterWithDeps(k kongBodyContext, cfg *Config, client sidecarTransport) {
	// 0) Idempotent plugin-side dedup (review-standards §5.2).
	if committed, _ := k.GetSharedString(CtxKeyCommitted); committed == "1" {
		return
	}

	// 1) Reservation id presence is the access-phase ALLOW marker.
	//    Missing → access either DENY'd or fail-opened. Either way
	//    no commit is owed (review-standards §5.3).
	reservationID, _ := k.GetSharedString(CtxKeyReservationID)
	if reservationID == "" {
		return
	}

	// 2) Pull the chunk and accumulate. Kong calls body_filter
	//    repeatedly; we append to the shared buffer until the
	//    empty chunk arrives.
	chunk, err := k.GetChunk()
	if err != nil {
		k.LogErr("spendguard body_filter: get chunk: " + err.Error())
		return
	}
	upstreamStatus, _ := k.GetUpstreamStatus()
	if upstreamStatus > 0 {
		_ = k.SetShared(CtxKeyUpstreamStatus, fmt.Sprintf("%d", upstreamStatus))
	}

	prev, _ := k.GetSharedString(CtxKeyBodyBuffer)
	accumulated := appendChunk(prev, chunk)
	if !isFinalChunk(chunk) {
		// Persist + wait for the next chunk. We trade one extra
		// SetShared call per chunk for a stable end-of-body
		// signal; an empty chunk is Kong's terminator.
		if err := k.SetShared(CtxKeyBodyBuffer, accumulated); err != nil {
			k.LogErr("spendguard body_filter: persist buffer: " + err.Error())
		}
		return
	}

	// 3) End-of-body: parse + emit.
	provider, _ := k.GetSharedString(CtxKeyProvider)
	upstreamStatusStr, _ := k.GetSharedString(CtxKeyUpstreamStatus)
	upstreamStatusInt := parseStatus(upstreamStatusStr, upstreamStatus)

	// Mark committed FIRST so any duplicate body_filter invocation
	// (e.g. Kong's keepalive teardown) is short-circuited even if
	// the trace POST below times out (review-standards §5.2).
	_ = k.SetShared(CtxKeyCommitted, "1")

	timeoutMS := cfg.TimeoutMS
	if timeoutMS <= 0 {
		timeoutMS = defaultTimeoutMS
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Duration(timeoutMS)*time.Millisecond)
	defer cancel()

	// Branch on upstream status first — a 5xx skips the parse
	// step entirely and goes straight to RUN_ABORTED. The body
	// from a 5xx is typically an HTML error page from upstream
	// which would fail the JSON parse anyway, but the explicit
	// branch makes the audit row's "reason" attributable to the
	// upstream error rather than to our parser.
	if upstreamStatusInt >= 500 {
		emitTrace(ctx, k, client, reservationID, nil, "REJECTED",
			fmt.Sprintf("upstream_%d", upstreamStatusInt))
		return
	}

	usage, err := parseProviderUsage(provider, []byte(accumulated))
	if err != nil {
		// Per review-standards §5.4-5.5 a malformed or unknown
		// upstream body emits RUN_ABORTED so the reservation
		// releases instead of silently committing.
		k.LogWarn("spendguard body_filter: parse usage: " + err.Error())
		emitTrace(ctx, k, client, reservationID, nil, "REJECTED", "parse_failed")
		return
	}

	emitTrace(ctx, k, client, reservationID, &usage, "ACCEPTED", "")
}

// emitTrace POSTs /v1/trace with the supplied verdict. A failure
// here is logged but does NOT exit the request (review-standards
// §5.6 — the upstream response is already on its way back).
func emitTrace(ctx context.Context, k kongBodyContext, client sidecarTransport, reservationID string,
	usage *ProviderUsage, verdict, providerEventID string) {
	req := TraceRequestBody{
		ReservationID: reservationID,
		Outcome:       verdict,
	}
	if usage != nil {
		input := usage.InputTokens
		output := usage.OutputTokens
		req.InputTokens = &input
		req.OutputTokens = &output
		// Sidecar requires actual_amount_atomic for ACCEPTED;
		// SLICE 5+ wires Tier-1 pricing here. For SLICE 4 we
		// surface the total token count as a stand-in so the
		// CommitEstimated lane proceeds — the cost projection
		// stays in the sidecar where the pricing schema lives.
		amount := fmt.Sprintf("%d", uint64(input)+uint64(output))
		req.ActualAmountAtomic = &amount
		if usage.ResponseID != "" {
			id := usage.ResponseID
			req.ProviderEventID = &id
		}
	} else if providerEventID != "" {
		id := providerEventID
		req.ProviderEventID = &id
	}

	if _, err := client.Trace(ctx, req); err != nil {
		var sErr *SidecarError
		if errors.As(err, &sErr) {
			k.LogWarn(fmt.Sprintf("spendguard body_filter: trace status=%d code=%s msg=%s",
				sErr.Status, sErr.Code, sErr.Message))
			return
		}
		k.LogWarn("spendguard body_filter: trace: " + err.Error())
	}
}

// ProviderUsage is the normalised usage view extracted from a
// provider response. Both OpenAI and Anthropic map cleanly into
// this shape; future providers slot in by extending parseProviderUsage.
type ProviderUsage struct {
	InputTokens  uint32
	OutputTokens uint32
	ResponseID   string
}

// parseProviderUsage extracts (input, output) tokens + response id
// from the upstream body. The function is provider-aware to keep
// the JSON path lookup cheap; an unknown provider returns an error
// per review-standards §5.4.
func parseProviderUsage(provider string, body []byte) (ProviderUsage, error) {
	if len(body) == 0 {
		return ProviderUsage{}, errors.New("empty response body")
	}
	switch strings.ToLower(provider) {
	case string(ProviderOpenAI), "":
		// Empty provider is treated as OpenAI by default — most
		// downstream Kong deployments target /v1/chat/completions
		// and the body shape carries enough disambiguation in the
		// `object` field that we accept either parser.
		return parseOpenAIUsage(body)
	case string(ProviderAnthropic):
		return parseAnthropicUsage(body)
	default:
		return ProviderUsage{}, fmt.Errorf("unknown provider %q", provider)
	}
}

// parseOpenAIUsage handles the `{"usage": {"prompt_tokens", "completion_tokens"}}`
// shape. `id` field carries the response id used by canonical
// ingest for dedup.
func parseOpenAIUsage(body []byte) (ProviderUsage, error) {
	var parsed struct {
		ID    string `json:"id"`
		Usage struct {
			PromptTokens     uint32 `json:"prompt_tokens"`
			CompletionTokens uint32 `json:"completion_tokens"`
		} `json:"usage"`
	}
	if err := json.Unmarshal(body, &parsed); err != nil {
		return ProviderUsage{}, fmt.Errorf("openai unmarshal: %w", err)
	}
	if parsed.Usage.PromptTokens == 0 && parsed.Usage.CompletionTokens == 0 {
		// Missing usage block is a parse failure per
		// review-standards §5.5 — we'd rather release than
		// commit silently with zeros.
		return ProviderUsage{}, errors.New("openai response missing usage block")
	}
	return ProviderUsage{
		InputTokens:  parsed.Usage.PromptTokens,
		OutputTokens: parsed.Usage.CompletionTokens,
		ResponseID:   parsed.ID,
	}, nil
}

// parseAnthropicUsage handles the `{"usage": {"input_tokens", "output_tokens"}}`
// shape; `id` carries the message id.
func parseAnthropicUsage(body []byte) (ProviderUsage, error) {
	var parsed struct {
		ID    string `json:"id"`
		Usage struct {
			InputTokens  uint32 `json:"input_tokens"`
			OutputTokens uint32 `json:"output_tokens"`
		} `json:"usage"`
	}
	if err := json.Unmarshal(body, &parsed); err != nil {
		return ProviderUsage{}, fmt.Errorf("anthropic unmarshal: %w", err)
	}
	if parsed.Usage.InputTokens == 0 && parsed.Usage.OutputTokens == 0 {
		return ProviderUsage{}, errors.New("anthropic response missing usage block")
	}
	return ProviderUsage{
		InputTokens:  parsed.Usage.InputTokens,
		OutputTokens: parsed.Usage.OutputTokens,
		ResponseID:   parsed.ID,
	}, nil
}

// appendChunk concatenates the prior buffer with the new chunk
// without re-allocating when possible.
func appendChunk(prev string, chunk []byte) string {
	if len(prev) == 0 {
		return string(chunk)
	}
	var b strings.Builder
	b.Grow(len(prev) + len(chunk))
	b.WriteString(prev)
	b.Write(chunk)
	return b.String()
}

// isFinalChunk follows Kong's body_filter contract: a zero-length
// chunk signals end-of-body.
func isFinalChunk(chunk []byte) bool { return len(chunk) == 0 }

// parseStatus tries the persisted shared value first then falls
// back to the live PDK status. Defaults to 200 when both are zero;
// 200 is the only safe "looks like success" default — using 0 here
// would let an upstream 502 slip past the §5.4 RUN_ABORTED branch.
func parseStatus(stored string, live int) int {
	if stored != "" {
		var v int
		_, err := fmt.Sscanf(stored, "%d", &v)
		if err == nil && v > 0 {
			return v
		}
	}
	if live > 0 {
		return live
	}
	return 200
}
