// Package main — Kong BodyFilter phase commit flow (D09 SLICE 4).
//
// Per `docs/specs/coverage/D09_kong_ai_gateway/design.md` §3.3:
//
//  1. Kong has already buffered the full upstream response body by
//     the time `body_filter` runs (we require `request_buffering:
//     true` and rely on Kong's response buffering for AI traffic).
//     `kong.service.response.get_raw_body` returns the ENTIRE
//     buffered body in a single call — NOT incremental chunks — so
//     we read it once and commit immediately rather than waiting for
//     an empty-chunk terminator that the PDK never sends. The
//     plugin-side dedup flag (CtxKeyCommitted) makes the commit fire
//     exactly once even though Kong re-invokes `body_filter` on
//     teardown (review-standards §5.1/§5.2).
//  2. On the (single) full-body read we parse the provider usage:
//     * OpenAI: `{"usage":{"prompt_tokens","completion_tokens"}}`
//     * Anthropic: `{"usage":{"input_tokens","output_tokens"}}`
//  3. POST /v1/trace with `LLM_CALL_POST.SUCCESS` (mapped to
//     ACCEPTED on the HTTP wire) carrying:
//     * the reservation_id stashed by `access.go`,
//     * the provider-reported token counts,
//     * the upstream response id for provider-side dedup.
//  4. Upstream 5xx, empty body, or unparseable body → POST /v1/trace
//     with REJECTED so the reservation is released (never silently
//     left to TTL-sweep).
//
// Idempotency: the sidecar handles ledger-side dedup on
// reservation_id (review-standards §5.2). The plugin-side dedup
// flag stops a second Trace call inside the same Kong request when
// body_filter fires more than once on the same logical end-of-body.
//
// Per review-standards §5.6 a sidecar timeout on the commit lane is
// logged but does not exit the request — the upstream response is
// already on its way back to the client. We do, however, run a
// bounded in-request retry on the trace POST (within the configured
// TimeoutMS budget) before giving up, so a single transient sidecar
// blip does not silently drop a realized-spend commit and leave the
// reservation to expire un-counted. The commit is idempotent on
// reservation_id (ledger SP + in-process IdempotencyCache), so the
// retry cannot double-commit. Test
// `body_filter_commit_timeout_does_not_short_circuit` exercises the
// non-exit path; `body_filter_commit_retries_then_succeeds` exercises
// the bounded retry.

package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/Kong/go-pdk"
	"github.com/Kong/go-pdk/server/kong_plugin_protocol"
)

// CtxKeyCommitted is the plugin-side dedup flag from
// review-standards §5.2 — once we've fired Trace for this request,
// subsequent body_filter invocations short-circuit silently.
const CtxKeyCommitted = "spendguard_committed"

// kongBodyContext is the BodyFilter analogue of kongContext. We
// only need a handful of PDK touchpoints + log.
type kongBodyContext interface {
	GetSharedString(key string) (string, error)
	SetShared(key string, value interface{}) error
	// GetFullBody returns the ENTIRE buffered upstream response body
	// in a single call. Kong's `kong.service.response.get_raw_body`
	// does not stream chunks — it returns the whole buffered body (or
	// a temp-file path for oversized bodies). The adapter resolves the
	// temp-file branch so callers always see the materialized bytes.
	GetFullBody() ([]byte, error)
	GetUpstreamStatus() (int, error)
	LogWarn(msg string)
	LogErr(msg string)
}

type pdkBodyAdapter struct{ k *pdk.PDK }

func (a *pdkBodyAdapter) GetSharedString(k string) (string, error) {
	return a.k.Ctx.GetSharedString(k)
}
func (a *pdkBodyAdapter) SetShared(k string, v interface{}) error { return a.k.Ctx.SetShared(k, v) }

// GetFullBody reads the full buffered service response body. The
// go-pdk `Response.GetRawBody()` helper silently discards the
// temp-file (`body_filepath`) branch and returns empty bytes for
// oversized bodies, which would make us commit a parse-failure
// RELEASE on a perfectly good large response. To stay fail-closed and
// accurate we drive the raw `RawBodyResult` ourselves and materialize
// the temp-file branch.
func (a *pdkBodyAdapter) GetFullBody() ([]byte, error) {
	out := new(kong_plugin_protocol.RawBodyResult)
	if err := a.k.ServiceResponse.Ask(`kong.service.response.get_raw_body`, nil, out); err != nil {
		return nil, err
	}
	return resolveRawBody(out)
}

// resolveRawBody materializes a Kong `RawBodyResult` (the real PDK
// wire type) into bytes, handling all three oneof branches the PDK can
// send: inline content, an oversized-body temp-file path, and an
// error. The go-pdk `Response.GetRawBody()` helper collapses this to
// `out.GetContent()` and silently drops the temp-file branch, which
// would make us treat a perfectly good large response as an empty
// body. Kept as a pure helper so it can be tested against the actual
// protobuf type without a live Kong (the synthetic chunk-queue mock
// it replaces did not match the real contract).
func resolveRawBody(out *kong_plugin_protocol.RawBodyResult) ([]byte, error) {
	switch x := out.Kind.(type) {
	case *kong_plugin_protocol.RawBodyResult_Content:
		return x.Content, nil
	case *kong_plugin_protocol.RawBodyResult_BodyFilepath:
		// Oversized body spilled to a temp file by Kong. Read it back
		// so usage parsing sees the real bytes.
		return os.ReadFile(x.BodyFilepath)
	case *kong_plugin_protocol.RawBodyResult_Error:
		return nil, errors.New(x.Error)
	default:
		return out.GetContent(), nil
	}
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

	// 2) Read the FULL buffered response body in one shot. Kong's
	//    `get_raw_body` returns the entire body (or a temp-file path
	//    for oversized bodies, resolved by the adapter) — it does NOT
	//    stream chunks, so there is no empty-chunk terminator to wait
	//    for. The dedup flag below (set BEFORE the trace POST) is what
	//    guarantees the commit fires exactly once across Kong's
	//    teardown re-invocations (review-standards §5.1/§5.2).
	body, err := k.GetFullBody()
	if err != nil {
		// We could not read the body, so we cannot know the realized
		// usage. Release the reservation rather than leaving it to
		// TTL-sweep (fail closed toward not over-counting, but the
		// reservation does not leak as a permanent hold).
		k.LogWarn("spendguard body_filter: get body: " + err.Error())
		commitOnce(k, cfg, client, reservationID, nil, "REJECTED", "body_read_failed")
		return
	}

	// 3) Resolve provider + upstream status, then parse + emit.
	provider, _ := k.GetSharedString(CtxKeyProvider)
	upstreamStatus, _ := k.GetUpstreamStatus()
	upstreamStatusInt := normalizeUpstreamStatus(upstreamStatus)

	// Branch on upstream status first — a 5xx skips the parse
	// step entirely and goes straight to RUN_ABORTED. The body
	// from a 5xx is typically an HTML error page from upstream
	// which would fail the JSON parse anyway, but the explicit
	// branch makes the audit row's "reason" attributable to the
	// upstream error rather than to our parser.
	if upstreamStatusInt >= 500 {
		commitOnce(k, cfg, client, reservationID, nil, "REJECTED",
			fmt.Sprintf("upstream_%d", upstreamStatusInt))
		return
	}

	usage, err := parseProviderUsage(provider, body)
	if err != nil {
		// Per review-standards §5.4-5.5 a malformed or unknown
		// upstream body (including an empty body) emits RUN_ABORTED so
		// the reservation releases instead of silently committing.
		k.LogWarn("spendguard body_filter: parse usage: " + err.Error())
		commitOnce(k, cfg, client, reservationID, nil, "REJECTED", "parse_failed")
		return
	}

	commitOnce(k, cfg, client, reservationID, &usage, "ACCEPTED", "")
}

// commitOnce sets the plugin-side dedup flag BEFORE firing the trace
// POST and then emits the trace. The flag-before-POST ordering is
// deliberate and load-bearing: Kong re-invokes body_filter on
// teardown, and the flag must already be set so the re-entry
// short-circuits at the top of runBodyFilterWithDeps even if the POST
// below is still in flight or has just failed (review-standards §5.2).
// emitTrace runs a bounded in-request retry so a single transient
// sidecar blip does not silently drop the commit; the commit is
// idempotent on reservation_id so the retry cannot double-count.
func commitOnce(k kongBodyContext, cfg *Config, client sidecarTransport, reservationID string,
	usage *ProviderUsage, verdict, providerEventID string) {
	// Mark committed FIRST so any duplicate body_filter invocation
	// (e.g. Kong's keepalive teardown) is short-circuited even if the
	// trace POST below times out (review-standards §5.2).
	_ = k.SetShared(CtxKeyCommitted, "1")

	timeoutMS := cfg.TimeoutMS
	if timeoutMS <= 0 {
		timeoutMS = defaultTimeoutMS
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Duration(timeoutMS)*time.Millisecond)
	defer cancel()

	emitTrace(ctx, k, client, reservationID, usage, verdict, providerEventID)
}

// commitMaxAttempts bounds the in-request retry on the commit lane.
// A transient sidecar blip (timeout, connection reset, 5xx) would
// otherwise drop a realized-spend commit and leave the reservation to
// TTL-sweep un-counted. The commit is idempotent on reservation_id
// (ledger SP per Stage 7 §11 + in-process IdempotencyCache) so a retry
// cannot double-commit. We cap attempts so the retry loop always
// stays inside the caller's TimeoutMS budget and never blocks the
// response already in flight (review-standards §5.6).
const commitMaxAttempts = 3

// commitRetryBackoff is the short pause between commit attempts. Kept
// tiny so 3 attempts fit comfortably inside a typical TimeoutMS; the
// shared ctx deadline is the hard ceiling regardless.
const commitRetryBackoff = 20 * time.Millisecond

// emitTrace POSTs /v1/trace with the supplied verdict, with a bounded
// retry on transient failure. A persistent failure is logged but does
// NOT exit the request (review-standards §5.6 — the upstream response
// is already on its way back). The committed flag is left set by the
// caller so re-entry still dedups even if every attempt here fails.
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

	var lastErr error
	for attempt := 1; attempt <= commitMaxAttempts; attempt++ {
		if _, err := client.Trace(ctx, req); err == nil {
			return
		} else {
			lastErr = err
		}
		// Out of budget? Stop — the response is already going back to
		// the client and the shared ctx deadline is the hard ceiling.
		if ctx.Err() != nil {
			break
		}
		if attempt < commitMaxAttempts {
			select {
			case <-ctx.Done():
				lastErr = ctx.Err()
			case <-time.After(commitRetryBackoff):
			}
			if ctx.Err() != nil {
				break
			}
		}
	}

	// Every attempt failed. The commit is idempotent on
	// reservation_id, so the sidecar's TTL-sweep companion (or a
	// future durable outbox, which per review-standards §1.1/§1.2 must
	// live sidecar-side, NOT in this translation-layer plugin) remains
	// the backstop. Log loudly so the dropped commit is alertable.
	var sErr *SidecarError
	if errors.As(lastErr, &sErr) {
		k.LogWarn(fmt.Sprintf("spendguard body_filter: trace failed after %d attempts status=%d code=%s msg=%s",
			commitMaxAttempts, sErr.Status, sErr.Code, sErr.Message))
		return
	}
	k.LogWarn(fmt.Sprintf("spendguard body_filter: trace failed after %d attempts: %v",
		commitMaxAttempts, lastErr))
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

// normalizeUpstreamStatus normalizes the live PDK upstream status.
// Defaults to 200 when the PDK reports zero; 200 is the only safe
// "looks like success" default — using 0 here would let an upstream
// 502 slip past the §5.4 RUN_ABORTED branch.
func normalizeUpstreamStatus(live int) int {
	if live > 0 {
		return live
	}
	return 200
}
