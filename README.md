# Duihua AI Services

Duihua AI Services is an OpenAI API-compatible platform for serving open-source LLMs and other AI models on Kubernetes with Helm.

## Architecture

- **Gateway (Rust, Axum)**: Provides OpenAI-compatible endpoints (`/v1/models`, `/v1/chat/completions`) and proxies requests to a model runtime.
- **Inference runtime**: Optional bundled `vllm/vllm-openai` deployment for OSS model hosting.
- **Kubernetes-first deployment**: Packaged as a cloud-provider-neutral Helm chart.

## Repository layout

- `services/gateway`: Rust gateway service.
- `charts/duihua-ai-services`: Helm chart for full deployment.
- `docs/`: Operational guidance.

## Quick start

### 1) Build gateway image

```bash
cd services/gateway
cargo build --release
```

Build and publish a container image (example):

```bash
docker build -t ghcr.io/<org>/duihua-gateway:0.1.0 .
docker push ghcr.io/<org>/duihua-gateway:0.1.0
```

### 2) Deploy with Helm

```bash
helm upgrade --install duihua charts/duihua-ai-services \
  --set gateway.image.repository=ghcr.io/<org>/duihua-gateway \
  --set gateway.image.tag=0.1.0
```

### 3) Call the API

```bash
kubectl port-forward svc/duihua-duihua-ai-services-gateway 8080:80
curl http://127.0.0.1:8080/v1/models
```

## Cloud-provider independence

The chart is Kubernetes-native and avoids cloud-specific resources by default.

Optional cloud integrations (e.g., AWS EBS CSI, load balancers, IAM roles for service accounts) can be added through values overrides as needed.

## Model configuration

- The default gateway model is `google/gemma-4-31B-it` (configurable via `gateway.env.defaultModel`).
- Configure one or more inference runtimes with `inference.models`.
- When `inference.enabled=true`, the chart creates one vLLM Deployment/Service per model and the gateway routes requests by requested model ID.
