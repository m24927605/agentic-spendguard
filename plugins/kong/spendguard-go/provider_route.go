// Package main — provider routing + body-shape detection (D09 SLICE 3).
//
// Per `docs/specs/coverage/D09_kong_ai_gateway/design.md` §3.5 the v1
// plugin covers OpenAI-shaped (`/v1/chat/completions`) and
// Anthropic-shaped (`/v1/messages`) payloads. This file owns the
// detection rules; review-standards §1.5 prohibits re-implementing
// `resolve_model_id` here. We MUST stay a string-passing layer:
// detect the body shape, pull the `model` string, hand it to the
// sidecar via the `/v1/tokenize` `provider` + `model` fields, and
// let the sidecar's `spendguard-provider-routing` crate do the real
// lookup.
//
// Anti-scope:
//
//   - No Bedrock SigV4 mutation (anti-scope §5).
//   - No SSE streaming detection in v1 — `stream: true` requests are
//     still tokenized + decided; we just commit at end-of-body
//     instead of per-chunk (design §3.3).
//   - No upstream cost mapping; the plugin's only job is to forward
//     enough context for the sidecar to do that mapping.

package main

import (
	"encoding/json"
	"fmt"
	"strings"
)

// ProviderKind enumerates the wire shapes the v1 plugin recognises.
// `ProviderUnknown` is the fail-closed sentinel; callers MUST refuse
// to proceed with `ProviderUnknown` rather than guessing a shape.
type ProviderKind string

const (
	ProviderUnknown   ProviderKind = ""
	ProviderOpenAI    ProviderKind = "openai"
	ProviderAnthropic ProviderKind = "anthropic"
)

// String implements fmt.Stringer for logging.
func (p ProviderKind) String() string { return string(p) }

// DetectedRequest is the parsed view of the upstream request body
// the Access hook hands to the sidecar /v1/tokenize call.
type DetectedRequest struct {
	Provider ProviderKind
	// Model id verbatim from the request body. We do NOT resolve it
	// to a `ProviderKind` server-side; that lives in the sidecar's
	// provider-routing crate (review-standards §1.5).
	Model string
	// Flattened prompt — concatenated message contents in arrival
	// order. The sidecar tokenizer re-tokenizes per-message; this
	// flat string is just a wire-shape conveyance.
	Prompt string
	// Streaming hint. SLICE 3 logs but does not branch on this; v2
	// SSE budget enforcement will use it.
	Stream bool
}

// DetectProvider inspects the request body and the request path
// to decide which provider wire shape we are looking at. Both inputs
// matter: `path` lets us pick a default provider for the rare case
// the body is too small to disambiguate; the body is the source of
// truth (review-standards §4.1: "parsed exactly once").
//
// Returns ProviderUnknown + an error when the body is neither shape;
// callers MUST translate this to a 400 to the downstream client
// (review-standards §4.6 — provider detection failure is a *client*
// error, not a server error).
func DetectProvider(path string, body []byte) (DetectedRequest, error) {
	if len(body) == 0 {
		return DetectedRequest{}, fmt.Errorf("spendguard: empty request body")
	}

	// Cheap shape detection: unmarshal into a flexible struct that
	// captures both OpenAI and Anthropic top-level keys. We avoid
	// `json.RawMessage` reflection on the hot path — Kong calls
	// Access on every request and the body is already in memory.
	var probe struct {
		// OpenAI chat: `{"model": "...", "messages": [{"role", "content"}]}`
		Model string `json:"model"`
		// Anthropic: same `messages` key, but `system` separate +
		// `max_tokens` required. We use the presence of
		// `max_tokens` (required by Anthropic, optional in OpenAI)
		// as the disambiguator alongside the path.
		MaxTokens *int            `json:"max_tokens"`
		Messages  json.RawMessage `json:"messages"`
		System    json.RawMessage `json:"system"`
		Stream    bool            `json:"stream"`
	}
	if err := json.Unmarshal(body, &probe); err != nil {
		return DetectedRequest{}, fmt.Errorf("spendguard: body is not JSON: %w", err)
	}
	if probe.Model == "" {
		return DetectedRequest{}, fmt.Errorf("spendguard: missing 'model' in request body")
	}
	if len(probe.Messages) == 0 {
		return DetectedRequest{}, fmt.Errorf("spendguard: missing 'messages' in request body")
	}

	provider := classifyProvider(path, probe.Model, probe.MaxTokens, probe.System)
	if provider == ProviderUnknown {
		return DetectedRequest{}, fmt.Errorf("spendguard: unrecognised provider shape (path=%q model=%q)", path, probe.Model)
	}

	prompt, err := flattenMessages(provider, probe.Messages, probe.System)
	if err != nil {
		return DetectedRequest{}, fmt.Errorf("spendguard: flatten messages: %w", err)
	}

	return DetectedRequest{
		Provider: provider,
		Model:    probe.Model,
		Prompt:   prompt,
		Stream:   probe.Stream,
	}, nil
}

// classifyProvider picks the provider shape based on path first
// (Kong routes are operator-controlled and high-trust), then on body
// content. Anthropic models are `claude-*`, OpenAI models are
// `gpt-*` / `o1-*` / `o3-*` / etc. Unknown model + ambiguous path
// returns ProviderUnknown so the caller fails closed.
func classifyProvider(path, model string, maxTokens *int, system json.RawMessage) ProviderKind {
	// Path-based disambiguation. Kong's ai-proxy uses these paths
	// verbatim; if the operator points a route at our plugin and
	// `ai-proxy` is downstream, the path tells us the upstream
	// provider before we look at the body.
	if strings.HasSuffix(path, "/v1/chat/completions") {
		return ProviderOpenAI
	}
	if strings.HasSuffix(path, "/v1/messages") {
		return ProviderAnthropic
	}

	// Body-based fallback. Model prefix is the canonical
	// disambiguator; both providers publish stable prefixes.
	lower := strings.ToLower(model)
	switch {
	case strings.HasPrefix(lower, "gpt-"),
		strings.HasPrefix(lower, "o1-"),
		strings.HasPrefix(lower, "o3-"),
		strings.HasPrefix(lower, "chatgpt-"):
		return ProviderOpenAI
	case strings.HasPrefix(lower, "claude-"):
		return ProviderAnthropic
	}

	// Shape heuristic — Anthropic requires `max_tokens` and
	// commonly carries a top-level `system` field; OpenAI does not.
	if maxTokens != nil && len(system) > 0 {
		return ProviderAnthropic
	}
	return ProviderUnknown
}

// flattenMessages concatenates message contents into a single
// newline-joined prompt for the sidecar tokenizer. Both providers
// use the same `[{role, content}]` shape; Anthropic's `system` lives
// at the top level so we prepend it.
func flattenMessages(provider ProviderKind, messagesJSON json.RawMessage, systemJSON json.RawMessage) (string, error) {
	var messages []struct {
		Role    string          `json:"role"`
		Content json.RawMessage `json:"content"`
	}
	if err := json.Unmarshal(messagesJSON, &messages); err != nil {
		return "", fmt.Errorf("messages parse: %w", err)
	}

	var b strings.Builder
	if provider == ProviderAnthropic && len(systemJSON) > 0 {
		// system can be a string or [{type, text}]; handle both.
		var sysStr string
		if err := json.Unmarshal(systemJSON, &sysStr); err == nil && sysStr != "" {
			b.WriteString(sysStr)
			b.WriteByte('\n')
		}
	}

	for _, m := range messages {
		// Both providers accept content as either a plain string
		// or an array of structured content blocks. We try string
		// first, then walk the structured form.
		var s string
		if err := json.Unmarshal(m.Content, &s); err == nil {
			b.WriteString(s)
			b.WriteByte('\n')
			continue
		}
		var blocks []struct {
			Type string `json:"type"`
			Text string `json:"text"`
		}
		if err := json.Unmarshal(m.Content, &blocks); err == nil {
			for _, blk := range blocks {
				if blk.Type == "text" || blk.Type == "" {
					b.WriteString(blk.Text)
					b.WriteByte('\n')
				}
			}
			continue
		}
		// Unknown content shape — fall through silently. The
		// tokenizer's chars/4 fallback still produces a usable
		// count; we don't want to fail the whole request on a
		// non-text image block.
	}
	return strings.TrimRight(b.String(), "\n"), nil
}
