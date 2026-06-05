// Minimal OpenAI/Anthropic-compatible upstream for gateway kind smoke tests.
package main

import (
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"strings"
	"time"

	"github.com/google/uuid"
)

var (
	defaultModel     = envOr("DEFAULT_MODEL", "HuggingFaceTB/SmolLM2-135M-Instruct")
	slowDelaySeconds = envFloat("SLOW_DELAY_SECONDS", 30)
	slowMarkers      = []string{"otter", "long story"}
)

func envOr(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func envFloat(key string, fallback float64) float64 {
	v := os.Getenv(key)
	if v == "" {
		return fallback
	}
	var f float64
	if _, err := fmt.Sscanf(v, "%f", &f); err != nil {
		return fallback
	}
	return f
}

func shouldDelay(payload any) bool {
	raw, err := json.Marshal(payload)
	if err != nil {
		return false
	}
	text := strings.ToLower(string(raw))
	for _, marker := range slowMarkers {
		if strings.Contains(text, marker) {
			return true
		}
	}
	return false
}

func extractModel(payload map[string]any) string {
	model, _ := payload["model"].(string)
	if model != "" {
		return model
	}
	return defaultModel
}

func responseText(payload map[string]any) string {
	inputValue, _ := payload["input"].(string)
	if inputValue == "" {
		return "ok"
	}
	lowered := strings.ToLower(inputValue)
	if strings.Contains(lowered, "bye") {
		return "bye"
	}
	if strings.Contains(lowered, "hi") {
		return "hi"
	}
	return "ok"
}

func writeJSON(w http.ResponseWriter, status int, body any) {
	encoded, err := json.Marshal(body)
	if err != nil {
		http.Error(w, `{"error":"internal error"}`, http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Content-Length", fmt.Sprintf("%d", len(encoded)))
	w.WriteHeader(status)
	_, _ = w.Write(encoded)
}

type server struct{}

func (s *server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	log.Printf("%s - %q %s", r.RemoteAddr, r.Method, r.URL.Path)

	switch r.Method {
	case http.MethodGet:
		s.handleGet(w, r)
	case http.MethodPost:
		s.handlePost(w, r)
	default:
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "not found"})
	}
}

func (s *server) handleGet(w http.ResponseWriter, r *http.Request) {
	switch r.URL.Path {
	case "/health", "/healthz":
		writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
	case "/v1/models":
		writeJSON(w, http.StatusOK, map[string]any{
			"object": "list",
			"data": []map[string]string{
				{
					"id":       defaultModel,
					"object":   "model",
					"owned_by": "mock-vllm",
				},
			},
		})
	default:
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "not found"})
	}
}

func (s *server) handlePost(w http.ResponseWriter, r *http.Request) {
	raw, err := io.ReadAll(r.Body)
	if err != nil {
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": "invalid body"})
		return
	}
	if len(raw) == 0 {
		raw = []byte("{}")
	}

	var payload map[string]any
	if err := json.Unmarshal(raw, &payload); err != nil {
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": "invalid JSON or encoding"})
		return
	}

	if shouldDelay(payload) {
		time.Sleep(time.Duration(slowDelaySeconds * float64(time.Second)))
	}

	switch r.URL.Path {
	case "/v1/responses":
		s.handleResponses(w, payload)
	case "/v1/messages":
		s.handleMessages(w, payload)
	case "/v1/messages/count_tokens":
		writeJSON(w, http.StatusOK, map[string]int{"input_tokens": 12})
	case "/v1/responses/input_tokens":
		writeJSON(w, http.StatusOK, map[string]int{"input_tokens": 12})
	default:
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "not found"})
	}
}

func (s *server) handleResponses(w http.ResponseWriter, payload map[string]any) {
	model := extractModel(payload)
	text := responseText(payload)
	writeJSON(w, http.StatusOK, map[string]any{
		"id":     "resp_" + strings.ReplaceAll(uuid.New().String(), "-", ""),
		"object": "response",
		"status": "completed",
		"model":  model,
		"output": []map[string]any{
			{
				"type": "message",
				"role": "assistant",
				"content": []map[string]string{
					{"type": "output_text", "text": text},
				},
			},
		},
	})
}

func (s *server) handleMessages(w http.ResponseWriter, payload map[string]any) {
	model := extractModel(payload)
	writeJSON(w, http.StatusOK, map[string]any{
		"id":          "msg_" + strings.ReplaceAll(uuid.New().String(), "-", ""),
		"type":        "message",
		"role":        "assistant",
		"model":       model,
		"content":     []map[string]string{{"type": "text", "text": "hi"}},
		"stop_reason": "end_turn",
		"usage":       map[string]int{"input_tokens": 8, "output_tokens": 4},
	})
}

func main() {
	host := envOr("HOST", "0.0.0.0")
	port := envOr("PORT", "8000")
	addr := host + ":" + port

	log.Printf("mock-vllm listening on %s (default_model=%s)", addr, defaultModel)
	if err := http.ListenAndServe(addr, &server{}); err != nil {
		log.Fatal(err)
	}
}