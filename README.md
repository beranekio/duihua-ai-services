# Duihua AI Services

Duihua AI Services is an OpenAI API-compatible platform for serving open-source LLMs and other AI models on Kubernetes with Helm.

## Architecture

- **Gateway (Rust, Axum)**: Provides OpenAI-compatible endpoints (`/v1/models`, `/v1/chat/completions`, `/v1/responses`, `/v1/embeddings`) and proxies requests to a model runtime.
- **Inference runtime**: Optional bundled `vllm/vllm-openai` deployment for OSS model hosting.
- **Responses API store (Valkey)**: Persists completed Responses API objects and conversation input so follow-up calls do not depend on inference runtime memory.
- **Kubernetes-first deployment**: Packaged as a cloud-provider-neutral Helm chart.

## Repository layout

- `services/gateway`: Rust API gateway service.
- `services/background-worker`: Lean Rust worker for background Responses API Jobs.
- `services/common`: Shared Rust library used by the gateway and background worker.
- `charts/duihua-ai-services`: Helm chart for full deployment.
- `scripts/`: Local kind bootstrap and deployment helpers.
- `docs/`: Operational guidance.

## Quick start

### 1) Build gateway and background worker images

```bash
docker build -t ghcr.io/<org>/duihua-gateway:0.1.0 --file services/gateway/Dockerfile services
docker build -t ghcr.io/<org>/duihua-background-worker:0.1.0 --file services/background-worker/Dockerfile services
docker push ghcr.io/<org>/duihua-gateway:0.1.0
docker push ghcr.io/<org>/duihua-background-worker:0.1.0
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
  --set gateway.image.tag=0.1.0 \
  --set backgroundWorker.image.repository=ghcr.io/<org>/duihua-background-worker \
  --set backgroundWorker.image.tag=0.1.0
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
gateway:
  responsesApiStore:
    enabled: true
  env:
    responseIdStoreKeyPrefix: duihua:responses
    responseIdStoreTtlSeconds: "86400"
valkey:
  enabled: true
inference:
  autoscaling:
    default:
      replicas:
        min: 0
        max: 1
```

To use an external Valkey/Redis-compatible service instead, keep `valkey.enabled=false` and set `gateway.env.responseIdStoreUrl`.

For local Docker Compose, set `RESPONSES_API_STORE_ENABLED=true` when starting the stack to exercise persisted follow-up Responses API calls. Streaming responses are persisted after their `response.completed` event.

With the Helm chart, `background=true` Responses API requests are executed asynchronously by Kubernetes Jobs that run the dedicated `duihua-background-worker` image (synchronous upstream call, result written to Valkey). Enable both `gateway.responsesApiStore.enabled=true` and `valkey.enabled=true` (or an external response store URL). Background jobs are on by default when the store is enabled (`gateway.responsesApiStore.backgroundJobs.enabled`). The kind workflow (`values-kind.yaml`) enables the store, Valkey, and background jobs for local testing.

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

# 3) Build local gateway and background worker images and load them into kind
scripts/build-and-load-images.sh

# 4) Deploy Helm chart, restart gateway pods, and verify rollout status
scripts/deploy-kind.sh

# 5) Exercise the gateway Responses API store and background jobs
scripts/smoke-test-kind.sh
```

Or run the full workflow:

```bash
scripts/kind-local-up.sh
```

By default, this creates a kind cluster named `duihua-local`, installs the chart into namespace `duihua`, enables the bundled CPU vLLM inference deployment, and exposes the gateway at `http://127.0.0.1:8080` via the kind port mapping in `kind/cluster.yaml`.

After a local gateway image rebuild, `scripts/build-and-load-images.sh` and `scripts/deploy-kind.sh` restart the gateway Deployment so running pods load the new image even when Helm reuses the same `GATEWAY_IMAGE_TAG` (default `local`). Local kind scripts that talk to the cluster (`install-keda.sh`, `deploy-kind.sh`, and the gateway restart helper) target the same kind cluster via `KUBECTL_CONTEXT` (default `kind-${CLUSTER_NAME}`), including Helm `--kube-context`. Set `GATEWAY_ROLLOUT_RESTART=false` to skip automatic restarts (rollout status is still checked after deploy).

```bash
curl http://127.0.0.1:8080/healthz
curl http://127.0.0.1:8080/v1/models
curl http://127.0.0.1:8080/v1/responses \
  -H 'Content-Type: application/json' \
  -d '{"input":"Write one sentence about Kubernetes."}'
```

Useful environment variables:
- `CLUSTER_NAME` (default: `duihua-local`; kind scripts default `KUBECTL_CONTEXT` to `kind-${CLUSTER_NAME}`)
- `KUBECTL_CONTEXT` (optional override for `install-keda.sh`, `deploy-kind.sh`, and gateway restart/status)
- `KIND_CONFIG` (default: `kind/cluster.yaml`)
- `RELEASE_NAME` (default: `duihua`)
- `NAMESPACE` (default: `duihua`)
- `GATEWAY_IMAGE_REPO` (default: `duihua-gateway`)
- `GATEWAY_IMAGE_TAG` (default: `local`)
- `BACKGROUND_WORKER_IMAGE_REPO` (default: `duihua-background-worker`)
- `BACKGROUND_WORKER_IMAGE_TAG` (default: same as `GATEWAY_IMAGE_TAG`)
- `GATEWAY_ROLLOUT_RESTART` (default: `true`, used by `scripts/build-and-load-images.sh` and `scripts/deploy-kind.sh`)
- `INFERENCE_ENABLED` (default: `true`)
- `VALUES_FILE` (default: `charts/duihua-ai-services/values-kind.yaml`)
- `KEDA_NAMESPACE` (default: `keda`)
- `GATEWAY_BASE_URL` (default: `http://127.0.0.1:8080`, used by `scripts/smoke-test-kind.sh`)
- `DEFAULT_MODEL` (default: `HuggingFaceTB/SmolLM2-135M-Instruct`, used by `scripts/smoke-test-kind.sh`)
- `MOCK_VLLM_IMAGE` (default: `ghcr.io/beranekio/mock-vllm:latest`, used by CI mock upstream scripts)

## CI

- **Validate** (`.github/workflows/validate.yml`): Runs on PRs and pushes to `main`. Performs Rust formatting/clippy/tests, Hadolint on the gateway Dockerfile, `helm lint`, and `helm template` rendering.
- **Kind Integration** (`.github/workflows/kind-integration.yml`): On PRs/pushes that touch the chart, kind assets, scripts, or gateway (manual trigger also available). Creates a kind cluster (`duihua-ci`), runs `scripts/ci-kind-integration.sh` (KEDA, gateway image build, `ghcr.io/beranekio/mock-vllm:latest` upstream, Helm deploy with `values-kind-ci.yaml`, `scripts/smoke-test-kind.sh`).

CI uses the published [mock-vllm](https://github.com/beranekio/mock-vllm) image (`ghcr.io/beranekio/mock-vllm:latest`) instead of the bundled vLLM inference stack. Local kind workflows still use `values-kind.yaml` with real inference via `scripts/kind-local-up.sh`.
