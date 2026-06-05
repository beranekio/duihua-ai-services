#!/usr/bin/env python3
"""Minimal OpenAI/Anthropic-compatible upstream for gateway kind smoke tests."""

from __future__ import annotations

import json
import os
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

DEFAULT_MODEL = os.environ.get("DEFAULT_MODEL", "HuggingFaceTB/SmolLM2-135M-Instruct")
SLOW_DELAY_SECONDS = float(os.environ.get("SLOW_DELAY_SECONDS", "30"))
SLOW_MARKERS = ("otter", "long story")


def should_delay(payload: object) -> bool:
    text = json.dumps(payload).lower()
    return any(marker in text for marker in SLOW_MARKERS)


def extract_model(payload: dict) -> str:
    model = payload.get("model")
    if isinstance(model, str) and model:
        return model
    return DEFAULT_MODEL


def response_text(payload: dict) -> str:
    input_value = payload.get("input")
    if isinstance(input_value, str):
        lowered = input_value.lower()
        if "bye" in lowered:
            return "bye"
        if "hi" in lowered:
            return "hi"
    return "ok"


class MockVllmHandler(BaseHTTPRequestHandler):
    server_version = "MockVLLM/1.0"

    def log_message(self, fmt: str, *args: object) -> None:
        print(f"{self.address_string()} - {fmt % args}")

    def do_GET(self) -> None:
        if self.path in ("/health", "/healthz", "/v1/models"):
            if self.path == "/v1/models":
                body = {
                    "object": "list",
                    "data": [
                        {
                            "id": DEFAULT_MODEL,
                            "object": "model",
                            "owned_by": "mock-vllm",
                        }
                    ],
                }
                self._json_response(200, body)
                return
            self._json_response(200, {"status": "ok"})
            return
        self._json_response(404, {"error": "not found"})

    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length else b"{}"
        try:
            payload = json.loads(raw.decode("utf-8") or "{}")
        except json.JSONDecodeError:
            self._json_response(400, {"error": "invalid JSON"})
            return

        if should_delay(payload):
            time.sleep(SLOW_DELAY_SECONDS)

        if self.path == "/v1/responses":
            self._handle_responses(payload)
            return
        if self.path == "/v1/messages":
            self._handle_messages(payload)
            return
        if self.path == "/v1/messages/count_tokens":
            self._handle_count_tokens(payload)
            return
        if self.path == "/v1/responses/input_tokens":
            self._json_response(200, {"input_tokens": 12})
            return

        self._json_response(404, {"error": "not found"})

    def _handle_responses(self, payload: dict) -> None:
        model = extract_model(payload)
        text = response_text(payload)
        body = {
            "id": f"resp_{uuid.uuid4().hex}",
            "object": "response",
            "status": "completed",
            "model": model,
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text}],
                }
            ],
        }
        self._json_response(200, body)

    def _handle_messages(self, payload: dict) -> None:
        model = extract_model(payload)
        body = {
            "id": f"msg_{uuid.uuid4().hex}",
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": [{"type": "text", "text": "hi"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 8, "output_tokens": 4},
        }
        self._json_response(200, body)

    def _handle_count_tokens(self, payload: dict) -> None:
        _ = payload
        self._json_response(200, {"input_tokens": 12})

    def _json_response(self, status: int, body: dict) -> None:
        encoded = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)


def main() -> None:
    host = os.environ.get("HOST", "0.0.0.0")
    port = int(os.environ.get("PORT", "8000"))
    server = ThreadingHTTPServer((host, port), MockVllmHandler)
    print(f"mock-vllm listening on {host}:{port} (default_model={DEFAULT_MODEL})")
    server.serve_forever()


if __name__ == "__main__":
    main()