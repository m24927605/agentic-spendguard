// Tests for the provider routing detector. Covers the §3.5 wire
// shapes (OpenAI + Anthropic) and the explicit ProviderUnknown
// sentinel path so a future provider doesn't silently fall through.

package main

import (
	"encoding/json"
	"strings"
	"testing"
)

func mustJSON(t *testing.T, v interface{}) []byte {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func TestDetectProvider_OpenAIChatCompletionsByPath(t *testing.T) {
	body := mustJSON(t, map[string]interface{}{
		"model": "gpt-4o-mini",
		"messages": []map[string]string{
			{"role": "user", "content": "hi"},
		},
	})
	d, err := DetectProvider("/v1/chat/completions", body)
	if err != nil {
		t.Fatal(err)
	}
	if d.Provider != ProviderOpenAI {
		t.Fatalf("provider: want openai, got %s", d.Provider)
	}
	if d.Model != "gpt-4o-mini" {
		t.Fatalf("model: %s", d.Model)
	}
	if !strings.Contains(d.Prompt, "hi") {
		t.Fatalf("prompt missing user content: %q", d.Prompt)
	}
}

func TestDetectProvider_AnthropicByPath(t *testing.T) {
	body := mustJSON(t, map[string]interface{}{
		"model":      "claude-3-5-sonnet-20241022",
		"max_tokens": 1024,
		"system":     "You are helpful.",
		"messages": []map[string]string{
			{"role": "user", "content": "ping"},
		},
	})
	d, err := DetectProvider("/v1/messages", body)
	if err != nil {
		t.Fatal(err)
	}
	if d.Provider != ProviderAnthropic {
		t.Fatalf("provider: want anthropic, got %s", d.Provider)
	}
	if !strings.Contains(d.Prompt, "You are helpful") {
		t.Fatalf("system prompt not prefixed: %q", d.Prompt)
	}
	if !strings.Contains(d.Prompt, "ping") {
		t.Fatalf("user content missing: %q", d.Prompt)
	}
}

func TestDetectProvider_OpenAIByModelPrefixFallback(t *testing.T) {
	body := mustJSON(t, map[string]interface{}{
		"model": "gpt-4o",
		"messages": []map[string]string{
			{"role": "user", "content": "x"},
		},
	})
	// Path does not match; model prefix wins.
	d, err := DetectProvider("/random/path", body)
	if err != nil {
		t.Fatal(err)
	}
	if d.Provider != ProviderOpenAI {
		t.Fatalf("model-prefix fallback failed: %s", d.Provider)
	}
}

func TestDetectProvider_AnthropicByModelPrefixFallback(t *testing.T) {
	body := mustJSON(t, map[string]interface{}{
		"model":      "claude-haiku",
		"max_tokens": 256,
		"messages":   []map[string]string{{"role": "user", "content": "x"}},
	})
	d, err := DetectProvider("/proxy/anthropic", body)
	if err != nil {
		t.Fatal(err)
	}
	if d.Provider != ProviderAnthropic {
		t.Fatalf("claude-prefix fallback failed: %s", d.Provider)
	}
}

func TestDetectProvider_O1ModelPrefix(t *testing.T) {
	body := mustJSON(t, map[string]interface{}{
		"model":    "o1-preview",
		"messages": []map[string]string{{"role": "user", "content": "x"}},
	})
	d, err := DetectProvider("/whatever", body)
	if err != nil {
		t.Fatal(err)
	}
	if d.Provider != ProviderOpenAI {
		t.Fatalf("o1-* model prefix not recognised as OpenAI: %s", d.Provider)
	}
}

func TestDetectProvider_RejectsEmptyBody(t *testing.T) {
	if _, err := DetectProvider("/v1/chat/completions", nil); err == nil {
		t.Fatal("empty body must error")
	}
}

func TestDetectProvider_RejectsNonJSON(t *testing.T) {
	if _, err := DetectProvider("/v1/chat/completions", []byte("not json")); err == nil {
		t.Fatal("non-json body must error")
	}
}

func TestDetectProvider_RejectsMissingModel(t *testing.T) {
	body := mustJSON(t, map[string]interface{}{
		"messages": []map[string]string{{"role": "user", "content": "x"}},
	})
	if _, err := DetectProvider("/v1/chat/completions", body); err == nil {
		t.Fatal("missing model must error")
	}
}

func TestDetectProvider_RejectsMissingMessages(t *testing.T) {
	body := mustJSON(t, map[string]interface{}{
		"model": "gpt-4",
	})
	if _, err := DetectProvider("/v1/chat/completions", body); err == nil {
		t.Fatal("missing messages must error")
	}
}

func TestDetectProvider_UnknownProviderReturnsError(t *testing.T) {
	body := mustJSON(t, map[string]interface{}{
		"model":    "llama-3-70b",
		"messages": []map[string]string{{"role": "user", "content": "x"}},
	})
	// Unknown path + unknown model prefix → ProviderUnknown.
	if _, err := DetectProvider("/random", body); err == nil {
		t.Fatal("unknown provider must error")
	}
}

func TestDetectProvider_StructuredContentBlocks(t *testing.T) {
	// Anthropic + OpenAI both accept structured content arrays.
	body := []byte(`{
		"model": "claude-3-5-sonnet-20241022",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "hello from block one"},
				{"type": "text", "text": "and block two"}
			]}
		]
	}`)
	d, err := DetectProvider("/v1/messages", body)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(d.Prompt, "block one") || !strings.Contains(d.Prompt, "block two") {
		t.Fatalf("structured content blocks not flattened: %q", d.Prompt)
	}
}

func TestDetectProvider_StreamFlagPreserved(t *testing.T) {
	body := mustJSON(t, map[string]interface{}{
		"model":    "gpt-4o-mini",
		"stream":   true,
		"messages": []map[string]string{{"role": "user", "content": "stream me"}},
	})
	d, err := DetectProvider("/v1/chat/completions", body)
	if err != nil {
		t.Fatal(err)
	}
	if !d.Stream {
		t.Fatal("stream flag not propagated")
	}
}
