FROM golang:1.24 AS builder
WORKDIR /app
COPY go.mod go.sum ./
RUN go mod download
COPY main.go ./
RUN CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o mock-vllm .

FROM gcr.io/distroless/static-debian13:nonroot
WORKDIR /app
COPY --from=builder /app/mock-vllm /app/mock-vllm

ENV HOST=0.0.0.0
ENV PORT=8000
ENV DEFAULT_MODEL=HuggingFaceTB/SmolLM2-135M-Instruct

EXPOSE 8000
CMD ["/app/mock-vllm"]