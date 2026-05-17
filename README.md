# Duihua AI Services

Duihua AI Services is an OpenAI API-compatible platform for serving open-source LLMs and other AI models on Kubernetes with Helm.

## Architecture

- **Gateway (Rust, Axum)**: Provides OpenAI-compatible endpoints (`/v1/models`, `/v1/chat/completions`, `/v1/embeddings`) and proxies requests to a model runtime.
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

Install KEDA and the KEDA HTTP add-on first (one-time per cluster):

```bash
helm repo add kedacore https://kedacore.github.io/charts
helm repo update

helm upgrade --install keda kedacore/keda \
  --namespace keda \
  --create-namespace

helm upgrade --install keda-add-ons-http kedacore/keda-add-ons-http \
  --namespace keda \
  --set interceptor.responseHeaderTimeout=120s
```

Then deploy Duihua:

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

### KEDA autoscaling for model deployments (default)

Inference models always use KEDA HTTP autoscaling. By default they scale from `0` when requests arrive and scale back down after an idle period.

This feature uses [KEDA](https://keda.sh/) with the KEDA HTTP add-on (must already be installed in your cluster).

Default behavior can be tuned globally, and overridden per model:

```yaml
inference:
  autoscaling:
    default:
      targetPendingRequests: 25
      scaledownPeriod: 600
      replicas:
        min: 0
        max: 1
  models:
    - name: google/gemma-4-31B-it
      autoscaling:
        hosts:
          - api.example.com
        pathPrefixes:
          - /v1/chat/completions
```

The chart creates an `InterceptorRoute` and `ScaledObject` for every model and sets Deployment replicas to `autoscaling.replicas.min`.
Gateway model upstreams are routed through the KEDA HTTP interceptor proxy (`inference.autoscaling.interceptorProxyUrl`) so scale-to-zero models cold-start correctly.

If you need a model to stay warm, set its minimum replicas to `1` (or higher):

```yaml
inference:
  models:
    - name: my/latency-critical-model
      autoscaling:
        replicas:
          min: 1
          max: 2
```

## Local kind workflow scripts

The `scripts/` directory includes helper scripts to stand up a local kind environment and deploy this chart end-to-end:

```bash
# 1) Create kind cluster
scripts/create-kind-cluster.sh

# 2) Install/upgrade KEDA and KEDA HTTP add-on
scripts/install-keda.sh

# 3) Build local gateway image and load it into kind
scripts/build-and-load-images.sh

# 4) Deploy Helm chart and verify rollout status
scripts/deploy-kind.sh
```

Or run the full workflow:

```bash
scripts/kind-local-up.sh
```

Useful environment variables:
- `CLUSTER_NAME` (default: `duihua-local`)
- `GATEWAY_IMAGE_REPO` (default: `duihua-gateway`)
- `GATEWAY_IMAGE_TAG` (default: `local`)
- `RELEASE_NAME` (default: `duihua`)
- `NAMESPACE` (default: `default`)
- `INFERENCE_ENABLED` (default: `false` for local kind)
