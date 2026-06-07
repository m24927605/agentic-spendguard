// Package main — Kong Access phase reserve flow (D09 SLICE 3).
//
// Per `docs/specs/coverage/D09_kong_ai_gateway/design.md` §3.3:
//
//  1. Kong has already buffered the request body (we require
//     `request_buffering: true` on the route per design §3.3).
//  2. We pull the body once (review-standards §4.1: parse exactly
//     once).
//  3. Detect provider shape (provider_route.go).
//  4. POST /v1/tokenize → input_tokens.
//  5. POST /v1/decision with the model + tokens.
//  6. Translate the verdict:
//     * ALLOW → stash reservation_id in `kong.ctx.shared` under
//     the documented key `spendguard_reservation_id`
//     (review-standards §4.3).
//     * DENY → kong.response.exit(429, JSON{error,reason_codes})
//     so the literal SPENDGUARD_DENY appears in the audit grep
//     (review-standards §4.2).
//     * DEGRADE → honor `cfg.FailOpen`; default closed
//     (review-standards §1.6 + §4.4).
//
// The pure decision logic lives in `runAccessWithDeps` so unit
// tests can hammer it with a fake sidecar + the go-pdk test harness
// without spinning up a real listener.

package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/Kong/go-pdk"
)

// CtxKeyReservationID is the stable `kong.ctx.shared` key the body
// filter reads. Documented per review-standards §4.3. Do not rename
// without bumping the plugin major version — Lua fallback (SLICE 5)
// and downstream telemetry consumers grep for this string.
const CtxKeyReservationID = "spendguard_reservation_id"

// CtxKeyProvider stores the detected provider shape so the body
// filter can pick the right usage-parsing branch. Mirrors design
// §3.3 step 2.
const CtxKeyProvider = "spendguard_provider"

// CtxKeyDegraded marks the request as degrade-fail-open so the body
// filter knows to skip the commit (no reservation was minted).
const CtxKeyDegraded = "spendguard_degraded"

// kongContext is the subset of *pdk.PDK we touch in Access. Pulled
// into an interface so unit tests can mock it without the
// go-pdk/test scaffold; the production code passes the real PDK
// through `Access` (see main.go).
type kongContext interface {
	GetRawBody() ([]byte, error)
	GetPath() (string, error)
	GetHeader(name string) (string, error)
	SetShared(key string, value interface{}) error
	Exit(status int, body []byte, headers map[string][]string)
	LogWarn(msg string)
	LogErr(msg string)
}

// pdkAdapter bridges *pdk.PDK to kongContext. Keeps `main.Access`
// trivial and lets every assertion in this file run against an
// in-process fake.
type pdkAdapter struct{ k *pdk.PDK }

func (a *pdkAdapter) GetRawBody() ([]byte, error) { return a.k.Request.GetRawBody() }
func (a *pdkAdapter) GetPath() (string, error)    { return a.k.Request.GetPath() }
func (a *pdkAdapter) GetHeader(name string) (string, error) {
	return a.k.Request.GetHeader(name)
}
func (a *pdkAdapter) SetShared(k string, v interface{}) error { return a.k.Ctx.SetShared(k, v) }
func (a *pdkAdapter) Exit(status int, body []byte, headers map[string][]string) {
	a.k.Response.Exit(status, body, headers)
}
func (a *pdkAdapter) LogWarn(m string) { _ = a.k.Log.Warn(m) }
func (a *pdkAdapter) LogErr(m string)  { _ = a.k.Log.Err(m) }

// sidecarTransport is the minimum interface runAccess needs. Lets
// tests pass an in-process fake without touching net/http.
type sidecarTransport interface {
	Tokenize(ctx context.Context, req TokenizeRequest) (TokenizeResponse, error)
	Decision(ctx context.Context, req DecisionRequestBody) (DecisionResponseBody, error)
	Trace(ctx context.Context, req TraceRequestBody) (TraceAckBody, error)
}

// runAccess is the production entry point invoked by `main.Access`.
// SLICE 3 lazily builds the sidecar client because the Kong
// plugin-server hands us the config at every request; production
// callers should consider caching a single client per Config keyed
// on `SidecarURL + ClientCertPEM` hash. v1 takes the simple path —
// connection re-use is already provided by net/http's default
// transport.
func runAccess(k *pdk.PDK, cfg *Config) {
	client, err := newSidecarClient(cfg)
	if err != nil {
		// Misconfigured plugin → fail-closed regardless of
		// FailOpen. The operator gets a 503 with a stable code
		// they can grep for (review-standards §4.7).
		_ = k.Log.Err("spendguard access: " + err.Error())
		k.Response.Exit(http503, denyBody(`SPENDGUARD_FAIL_CLOSED`, "plugin misconfigured"), jsonHeaders())
		return
	}
	runAccessWithDeps(&pdkAdapter{k: k}, cfg, client)
}

// runAccessWithDeps is the unit-testable core. Splits exactly the
// way the spec § states:
//
//   - body once
//   - provider detection
//   - tokenize
//   - decision
//   - verdict branch
//
// On any pre-decision failure the function returns via
// failOpenOrDeny so the fail-closed default is centralised.
func runAccessWithDeps(k kongContext, cfg *Config, client sidecarTransport) {
	// 1) Pull body + path once (review-standards §4.1).
	body, err := k.GetRawBody()
	if err != nil {
		failOpenOrDeny(k, cfg, http502, "SPENDGUARD_BODY_READ", "body read failed: "+err.Error())
		return
	}
	path, err := k.GetPath()
	if err != nil {
		failOpenOrDeny(k, cfg, http502, "SPENDGUARD_PATH_READ", "path read failed: "+err.Error())
		return
	}

	// 2) Detect provider.
	detected, err := DetectProvider(path, body)
	if err != nil {
		// Provider detection failure is a *client* error
		// (review-standards §4.6): the upstream sent a body we
		// can't recognise as either OpenAI or Anthropic shape.
		// Translate to 400, not 503.
		_ = sendExit(k, http400, "SPENDGUARD_UNRECOGNISED_REQUEST", err.Error())
		return
	}

	// 3) Idempotency key — prefer the operator-supplied header
	//    (review-standards §4.8); fall back to a deterministic hash
	//    so a Kong retry from the same client still collapses.
	idemKey, err := k.GetHeader("Idempotency-Key")
	if err != nil {
		idemKey = ""
	}
	if idemKey == "" {
		idemKey = fmt.Sprintf("kong-auto-%x", hashBody(body))
	}

	// 4) Timeout budget. Per review-standards §4.5 we treat a
	//    sidecar timeout as DEGRADE not an Err.
	timeoutMS := cfg.TimeoutMS
	if timeoutMS <= 0 {
		timeoutMS = defaultTimeoutMS
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Duration(timeoutMS)*time.Millisecond)
	defer cancel()

	// 5) Tokenize.
	tokResp, err := client.Tokenize(ctx, TokenizeRequest{
		Provider: string(detected.Provider),
		Model:    detected.Model,
		Prompt:   detected.Prompt,
	})
	if err != nil {
		failOpenOrDeny(k, cfg, http503, "SPENDGUARD_TOKENIZE_UNREACHABLE", "tokenize: "+err.Error())
		return
	}

	// 6) Decision. claim_estimate_atomic is a placeholder while
	//    SLICE 5+ wires Tier 1/2 token-to-atomic conversion.
	//    Surface the token count to the sidecar verbatim — the
	//    contract evaluator's per-token cost rules use that value
	//    via the `prompt_class` / `model_class` strings.
	decisionReq := DecisionRequestBody{
		TenantID:            cfg.TenantID,
		ClaimEstimateAtomic: fmt.Sprintf("%d", tokResp.InputTokens),
		PromptClass:         "general",
		ModelClass:          fmt.Sprintf("%s/%s", detected.Provider, detected.Model),
		IdempotencyKey:      idemKey,
	}
	decResp, err := client.Decision(ctx, decisionReq)
	if err != nil {
		// Sidecar-side 409 IdempotencyConflict is a client error,
		// not a degrade path. Pass it through so a misbehaving
		// caller sees the conflict explicitly.
		var sErr *SidecarError
		if errors.As(err, &sErr) && sErr.Status == http409 {
			_ = sendExit(k, http409, "SPENDGUARD_IDEMPOTENCY_CONFLICT", sErr.Message)
			return
		}
		failOpenOrDeny(k, cfg, http503, "SPENDGUARD_DECISION_UNREACHABLE", "decision: "+err.Error())
		return
	}

	// 7) Verdict branch.
	switch decResp.Verdict {
	case "ALLOW":
		if decResp.ReservationID == "" {
			// Sidecar contract: ALLOW must carry a
			// reservation_id. An empty one means the
			// companion is mis-wired; fail closed.
			failOpenOrDeny(k, cfg, http503, "SPENDGUARD_RESERVATION_MISSING", "ALLOW without reservation_id")
			return
		}
		if err := k.SetShared(CtxKeyReservationID, decResp.ReservationID); err != nil {
			k.LogErr("spendguard: SetShared reservation_id: " + err.Error())
			// Failing to persist the reservation_id makes the
			// downstream commit impossible — fail closed.
			failOpenOrDeny(k, cfg, http503, "SPENDGUARD_CTX_WRITE", "ctx.shared write failed")
			return
		}
		_ = k.SetShared(CtxKeyProvider, string(detected.Provider))
	case "DENY":
		_ = sendExit(k, http429, "SPENDGUARD_DENY", denyMessage(decResp.ReasonCodes))
	case "DEGRADE":
		if cfg.FailOpen {
			_ = k.SetShared(CtxKeyDegraded, "1")
			k.LogWarn("spendguard: DEGRADE with fail_open=true; allowing call to proceed")
		} else {
			_ = sendExit(k, http503, "SPENDGUARD_DEGRADE", "guardrail degraded; fail-closed")
		}
	default:
		// Unknown verdict — fail closed (this should never
		// happen if the sidecar honors the wire shape).
		failOpenOrDeny(k, cfg, http503, "SPENDGUARD_UNKNOWN_VERDICT", "unknown verdict: "+decResp.Verdict)
	}
}

// failOpenOrDeny implements the §3.4 fail-policy split. When
// `FailOpen=true` the call is allowed to proceed but the operator
// sees a structured warn-log they can alert on. Otherwise we exit
// 503 with a stable code.
func failOpenOrDeny(k kongContext, cfg *Config, status int, code, msg string) {
	// Differentiate client-config from operator-config per
	// review-standards §4.4 — the log carries both so the
	// telemetry consumer can grep for either.
	k.LogWarn(fmt.Sprintf("spendguard degrade: code=%s fail_open=%t msg=%s", code, cfg.FailOpen, msg))
	if cfg.FailOpen {
		_ = k.SetShared(CtxKeyDegraded, "1")
		return
	}
	_ = sendExit(k, status, code, msg)
}

// sendExit emits a JSON body containing the stable error code
// (review-standards §4.2 — DENY response MUST be JSON).
func sendExit(k kongContext, status int, code, message string) error {
	body, err := json.Marshal(map[string]interface{}{
		"error":        message,
		"code":         code,
		"reason_codes": []string{code},
	})
	if err != nil {
		body = denyBody(code, message)
	}
	k.Exit(status, body, jsonHeaders())
	return nil
}

func denyBody(code, message string) []byte {
	// Manual fallback for the unlikely json.Marshal failure
	// above. Keeps the SPENDGUARD_DENY string visible for grep.
	return []byte(fmt.Sprintf(`{"error":%q,"code":%q,"reason_codes":[%q]}`, message, code, code))
}

func denyMessage(reasonCodes []string) string {
	if len(reasonCodes) == 0 {
		return "budget exceeded"
	}
	return "budget exceeded: " + joinReasonCodes(reasonCodes)
}

func joinReasonCodes(rc []string) string {
	if len(rc) == 0 {
		return ""
	}
	out := rc[0]
	for _, c := range rc[1:] {
		out += "," + c
	}
	return out
}

func jsonHeaders() map[string][]string {
	return map[string][]string{"Content-Type": {"application/json"}}
}

// hashBody is the deterministic fallback for Idempotency-Key
// missing. Uses a cheap FNV-1a so we don't pull in sha256 for a
// per-request hot path; review-standards §4.8 only requires
// determinism, not collision-resistance.
func hashBody(b []byte) uint64 {
	const (
		offset64 = uint64(14695981039346656037)
		prime64  = uint64(1099511628211)
	)
	h := offset64
	for _, c := range b {
		h ^= uint64(c)
		h *= prime64
	}
	return h
}

// HTTP status codes used by the access flow. Kept as const ints so
// the body builder can stay in lockstep with the integration test.
const (
	http400 = 400
	http409 = 409
	http429 = 429
	http502 = 502
	http503 = 503
)
