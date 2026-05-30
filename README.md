# Duihua AI Services

Duihua AI Services is an OpenAI API-compatible platform for serving open-source LLMs and other AI models on Kubernetes with Helm.

## Architecture

- **Gateway (Rust, Axum)**: Provides OpenAI-compatible endpoints (`/v1/models`, `/v1/chat/completions`, `/v1/responses`, `/v1/embeddings`) and proxies requests to a model runtime.
- **Inference runtime**: Optional bundled `vllm/vllm-openai` deployment for OSS model hosting.
- **Responses API store (Valkey)**: Persists completed Responses API objects and conversation input so follow-up calls do not depend on inference runtime memory.
- **Kubernetes-first deployment**: Packaged as a cloud-provider-neutral Helm chart.

## Repository layout

- `services/gateway`: Rust gateway service.
- `charts/duihua-ai-services`: Helm chart for full deployment.
- `scripts/`: Local kind bootstrap and deployment helpers.
- `docs/`: Operational guidance.

## Quick start

### 1) Build gateway image

```bash
docker build -t ghcr.io/<org>/duihua-gateway:0.1.0 services/gateway
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
  --namespace duihua \
  --create-namespace \
  --set gateway.image.repository=ghcr.io/<org>/duihua-gateway \
  --set gateway.image.tag=0.1.0
```

### 3) Call the API

```bash
kubectl port-forward -n duihua svc/duihua-duihua-ai-services-gateway 8080:80
curl http://127.0.0.1:8080/v1/models

curl http://127.0.0.1:8080/v1/responses \
  -H 'Content-Type: application/json' \
  -d '{"model":"google/gemma-4-31B-it","input":"Write one sentence about Kubernetes."}'
```

## Responses API store

The gateway can persist completed Responses API objects and their materialized conversation input in Valkey-compatible Redis storage. Follow-up creation requests with `previous_response_id` are expanded by the gateway into stateless upstream requests, while retrieval, deletion, and input-item requests are served directly from Valkey. The gateway does not enable or depend on vLLM's in-process Responses API store, so inference deployments can use multiple replicas and scale to zero between calls.

Response persistence is optional and disabled by default. When disabled, follow-up `{response_id}` requests return the same not-found error shape as vLLM instead of being forwarded to an inference deployment. Creation requests that explicitly set `store: false` are never persisted by the gateway.

To enable it with the chart-managed Valkey instance:

```yaml
inference:
  responsesApiStore:
    enabled: true
  autoscaling:
    default:
      replicas:
        min: 0
        max: 1
valkey:
  enabled: true
gateway:
  env:
    responseIdStoreKeyPrefix: duihua:responses
    responseIdStoreTtlSeconds: "86400"
```

To use an external Valkey/Redis-compatible service instead, keep `valkey.enabled=false` and set `gateway.env.responseIdStoreUrl`.

For local Docker Compose, set `RESPONSES_API_STORE_ENABLED=true` when starting the stack to exercise persisted follow-up Responses API calls. Streaming responses are persisted after their `response.completed` event. Background responses are not currently supported by the gateway-owned store.

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

The chart creates an `InterceptorRoute`, `ScaledObject`, and per-model proxy `Service` for every model and sets Deployment replicas to `autoscaling.replicas.min`.
Gateway model upstreams are routed through those per-model proxy Services, which resolve to the shared KEDA HTTP interceptor proxy (`inference.autoscaling.interceptorProxyUrl`, typically `http://keda-add-ons-http-interceptor-proxy.keda.svc.cluster.local:8080`), so scale-to-zero models cold-start correctly without rewriting the `/v1/...` request path.

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

By default, this creates a kind cluster named `duihua-local`, installs the chart into namespace `duihua`, enables the bundled CPU vLLM inference deployment, and exposes the gateway at `http://127.0.0.1:8080` via the kind port mapping in `kind/cluster.yaml`.

```bash
curl http://127.0.0.1:8080/healthz
curl http://127.0.0.1:8080/v1/models
curl http://127.0.0.1:8080/v1/responses \
  -H 'Content-Type: application/json' \
  -d '{"input":"Write one sentence about Kubernetes."}'
```

Useful environment variables:
- `CLUSTER_NAME` (default: `duihua-local`)
- `KIND_CONFIG` (default: `kind/cluster.yaml`)
- `RELEASE_NAME` (default: `duihua`)
- `NAMESPACE` (default: `duihua`)
- `GATEWAY_IMAGE_REPO` (default: `duihua-gateway`)
- `GATEWAY_IMAGE_TAG` (default: `local`)
- `INFERENCE_ENABLED` (default: `true`)
- `VALUES_FILE` (default: `charts/duihua-ai-services/values-kind.yaml`)
- `KEDA_NAMESPACE` (default: `keda`)

## CI

- **Validate** (`.github/workflows/validate.yml`): Runs on PRs and pushes to `main`. Performs Rust formatting/clippy/tests, Dockerfile build, `helm lint`, and `helm template` rendering.
- **Helm Kind Test** (`.github/workflows/helm-kind-test.yml`): Runs on PRs and pushes to `main` (also supports manual trigger). Creates a real kind cluster using `kind/cluster.yaml`, installs KEDA + the HTTP add-on, builds the gateway image, deploys the Helm chart with `values-kind.yaml`, waits for rollouts, and performs basic smoke tests against the gateway.

The kind test exercises the full installation path used by the local scripts in a real cluster. By default it runs with inference disabled for speed; the full configuration (including vLLM inference pods) can be tested via manual workflow dispatch.
