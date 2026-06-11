# Duihua AI Services

Duihua AI Services is an OpenAI API-compatible platform for serving open-source LLMs and other AI models on Kubernetes with Helm.

## Architecture

- **Gateway (Rust, Axum)**: Published as [beranekio/duihua-gateway](https://github.com/beranekio/duihua-gateway) and consumed via the `duihua-gateway` Helm subchart. Provides OpenAI-compatible endpoints and proxies requests to model runtimes.
- **Inference runtime**: Optional bundled `vllm/vllm-openai` deployment for OSS model hosting.
- **Responses API store (gRPC)**: Persists completed Responses API objects and conversation input via the `responses-api-store` gRPC service (Valkey/Redis-backed).
- **Background worker (Rust)**: Consumes a Valkey stream queue and completes `background=true` Responses API requests via synchronous upstream calls.
- **Kubernetes-first deployment**: Packaged as a cloud-provider-neutral Helm chart.

## Repository layout

- `services/background-worker`: Valkey stream consumer for background Responses API requests.
- Gateway source and image: [beranekio/duihua-gateway](https://github.com/beranekio/duihua-gateway) (OCI subchart `oci://ghcr.io/beranekio/charts/duihua-gateway`).

- `charts/duihua-ai-services`: Helm chart for full deployment.
- `scripts/`: Local kind bootstrap and deployment helpers.
- `docs/`: Operational guidance.

## Quick start

### 1) Build images

Gateway images are published from [beranekio/duihua-gateway](https://github.com/beranekio/duihua-gateway) (`ghcr.io/beranekio/duihua-gateway`). Build the background worker from this repo:

```bash
docker build -t ghcr.io/<org>/duihua-background-worker:0.1.0 --file services/background-worker/Dockerfile services
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
helm dependency update charts/duihua-ai-services
helm upgrade --install duihua charts/duihua-ai-services \
  --namespace duihua \
  --create-namespace \
  --set backgroundWorker.image.repository=ghcr.io/<org>/duihua-background-worker \
  --set backgroundWorker.image.tag=0.1.0 \
  --set duihua-gateway.env.modelUpstreams="google/gemma-4-31B-it=http://duihua-duihua-ai-services-inference-0-proxy:8080/v1"
```

When `inference.enabled=true` (the chart default), `duihua-gateway.env.modelUpstreams` must list each bundled model and its per-model inference proxy Service (`<release>-duihua-ai-services-inference-<index>-proxy`). Helm fails at render time if it is missing. For kind/local workflows, `scripts/deploy-kind.sh` computes and injects this mapping automatically.

### 3) Call the API

```bash
kubectl port-forward -n duihua svc/duihua-duihua-gateway 8080:80
curl http://127.0.0.1:8080/v1/models

curl http://127.0.0.1:8080/v1/responses \
  -H 'Content-Type: application/json' \
  -d '{"model":"google/gemma-4-31B-it","input":"Write one sentence about Kubernetes."}'
```

## Responses API store

The gateway can persist completed Responses API objects and their materialized conversation input through the `responses-api-store` gRPC service (Valkey/Redis-backed). Follow-up creation requests with `previous_response_id` are expanded by the gateway into stateless upstream requests, while retrieval, deletion, and input-item requests are served from the store service. The gateway does not enable or depend on vLLM's in-process Responses API store, so inference deployments can use multiple replicas and scale to zero between calls.

Response persistence is optional and disabled by default. When disabled, follow-up `{response_id}` requests return the same not-found error shape as vLLM instead of being forwarded to an inference deployment. Creation requests that explicitly set `store: false` are never persisted by the gateway.

To enable it with the chart-managed `responses-api-store` subchart and bundled Valkey:

```yaml
responsesApiStoreService:
  enabled: true

duihua-gateway:
  responsesApiStore:
    enabled: true
    endpoint: http://duihua-responses-api-store:50051
  env:
    responseIdStoreTtlSeconds: "86400"

responses-api-store:
  store:
    keyPrefix: responses-api-store:responses
    ttlSeconds: 86400
  valkey:
    enabled: true

inference:
  autoscaling:
    default:
      replicas:
        min: 0
        max: 1
```

Gateway and background worker receive `RESPONSES_API_STORE_ENDPOINT` via `duihua-gateway.responsesApiStore.endpoint` (set explicitly or use `scripts/deploy-kind.sh`, which wires the subchart Service). Redis connection settings belong to the `responses-api-store` subchart, not gateway env vars. When `duihua-gateway.responsesApiStore.enabled=true`, also enable `responsesApiStoreService.enabled=true` or set `duihua-gateway.responsesApiStore.endpoint` to an external gRPC URL; Helm fails at render time for unsupported combinations. Tune stale `in_progress` reconciliation via `responses-api-store.store.staleSeconds` (`backgroundWorker.staleSeconds` is a deprecated alias; Helm fails at render time when it is set to a value that differs from the subchart setting).

To use an external Valkey/Redis-compatible service instead, keep the subchart enabled, disable bundled Valkey, and point the store service at your cluster:

```yaml
responsesApiStoreService:
  enabled: true

responses-api-store:
  valkey:
    enabled: false
  redis:
    url: rediss://your-redis.example:6379/0
```

With external Redis above, keep the default `store-metrics` autoscaling driver: KEDA queries the responses-api-store HTTP metrics endpoint and does not need Valkey access. The legacy `redis-streams` driver still points at the chart-managed Valkey Service name when `responsesApiStoreService.enabled=true`, even if bundled Valkey is disabled; use `store-metrics` here or see issue #58 before opting into `redis-streams`.

For local Docker Compose, use `docker-compose.gateway-local.yml`. It pulls the published gateway and responses-api-store images from GHCR and does not require a local checkout of [beranekio/duihua-gateway](https://github.com/beranekio/duihua-gateway). The compose file starts the gateway only by default; add `--profile inference` for the bundled vLLM upstream and/or `--profile store` for the responses-api-store and Valkey services. Override `DUIHUA_GATEWAY_IMAGE` or `RESPONSES_API_STORE_IMAGE` to pin a specific tag. To hack on gateway source, use `docker compose` in the `duihua-gateway` repository instead.

With the Helm chart, `background=true` Responses API requests are enqueued on a Valkey stream (via the store service) and processed by the `duihua-background-worker` Deployment (synchronous upstream call per message, result written back through the store service). On rollout restart the worker drains in-flight jobs on SIGTERM before exit; set `backgroundWorker.terminationGracePeriodSeconds` above `backgroundWorker.upstreamTimeoutSeconds` plus `backgroundWorker.blockMs` and a safety margin (chart default 665s). The worker logs a recommended grace period at startup. Enable `responsesApiStoreService.enabled=true`, `duihua-gateway.responsesApiStore.enabled=true`, and `backgroundWorker.enabled=true`. The kind workflow (`values-kind.yaml`) enables the store subchart, bundled Valkey, background worker, queue settings, and KEDA store-metrics autoscaling for local end-to-end background completion testing.

### KEDA autoscaling for background workers (optional)

When `backgroundWorker.autoscaling.enabled=true`, the chart creates a KEDA `ScaledObject` on the worker Deployment. By default (`driver: store-metrics`), scaling reads the store's `workload` metric from `GET /metrics/background-queue?consumer_group=...` on the responses-api-store Service (`pending + in_progress` jobs). This uses KEDA's `metrics-api` scaler (KEDA core only; the HTTP add-on is not required). KEDA does not need Valkey credentials or stream keys.

```yaml
backgroundWorker:
  autoscaling:
    enabled: true
    driver: store-metrics
    jobsPerReplica: 5
    activationTargetValue: 0
    scaledownPeriod: 300
    replicas:
      min: 0   # scale-to-zero when idle
      max: 4

responses-api-store:
  metrics:
    enabled: true
    port: 8080
```

Set `replicas.min` to `1` or higher to keep at least one worker pod warm. Use `activationTargetValue: 0` (or `activationLagCount: 0`) so the first queued job wakes a worker. Tune `jobsPerReplica` or `lagCount` (average queue workload target per replica); lower values scale up sooner. The store computes `workload` from Redis Streams consumer-group stats, so Valkey/Redis **7+** is required for `store-metrics` as well as `redis-streams`. When autoscaling is enabled, KEDA owns replica counts, the chart omits `spec.replicas`, and `backgroundWorker.replicaCount` is ignored. With `replicas.min: 0`, the gateway ensures the stream consumer group via the responses-api-store gRPC service on startup so workers can claim jobs once scaled up.

Legacy `driver: redis-streams` remains available for migration. It scales from Redis Streams consumer-group lag directly and honors `lagCount` / `activationLagCount` before the newer key names. With bundled Valkey disabled, `redis-streams` does not automatically follow `responses-api-store.redis.url`; track issue #58 or disable the store subchart and set `duihua-gateway.responsesApiStore.redisAddress`. When the store subchart is disabled, set `backgroundWorker.autoscaling.metricsUrl` for the `store-metrics` driver.

## Cloud-provider independence

The chart is Kubernetes-native and avoids cloud-specific resources by default.

Optional cloud integrations (e.g., AWS EBS CSI, load balancers, IAM roles for service accounts) can be added through values overrides as needed.

## Model configuration

- The default gateway model is `google/gemma-4-31B-it` (configurable via `duihua-gateway.env.defaultModel`).
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
Gateway model upstreams are routed through those per-model proxy Services via `duihua-gateway.env.modelUpstreams` (see `values-kind.yaml` for an example), which resolve to the shared KEDA HTTP interceptor proxy (`inference.autoscaling.interceptorProxyUrl`, typically `http://keda-add-ons-http-interceptor-proxy.keda.svc.cluster.local:8080`), so scale-to-zero models cold-start correctly without rewriting the `/v1/...` request path.

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

# 3) Build the background worker image and load it into kind (gateway image comes from the duihua-gateway subchart)
scripts/build-and-load-images.sh

# 4) Deploy Helm chart, restart gateway pods, and verify rollout status
scripts/deploy-kind.sh

# 5) Exercise the gateway Responses API store and background worker queue
scripts/smoke-test-kind.sh
```

Or run the full workflow:

```bash
scripts/kind-local-up.sh
```

By default, this creates a kind cluster named `duihua-local`, installs the chart into namespace `duihua`, enables the bundled CPU vLLM inference deployment, and exposes the gateway at `http://127.0.0.1:8080` via the kind port mapping in `kind/cluster.yaml`.

After a local background-worker image rebuild, `scripts/build-and-load-images.sh` and `scripts/deploy-kind.sh` restart the background-worker Deployment so running pods load the new image even when Helm reuses the same image tag (default `local`). The gateway image is chosen by the pinned `duihua-gateway` OCI subchart and pulled from GHCR by the cluster. `scripts/deploy-kind.sh` also restarts the gateway Deployment after chart upgrades. Local kind scripts that talk to the cluster (`install-keda.sh`, `deploy-kind.sh`, and the rollout restart helpers) target the same kind cluster via `KUBECTL_CONTEXT` (default `kind-${CLUSTER_NAME}`), including Helm `--kube-context`. Set `GATEWAY_ROLLOUT_RESTART=false` or `BACKGROUND_WORKER_ROLLOUT_RESTART=false` to skip automatic restarts (rollout status is still checked after deploy). If a rollout times out while the Deployment is still Available with ready pods, set `ROLLOUT_STRICT=false` (or per-deployment `GATEWAY_ROLLOUT_STRICT` / `BACKGROUND_WORKER_ROLLOUT_STRICT`) to continue; the restart helpers print deployment conditions and events on failure.

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
- `GATEWAY_STORE_ENDPOINT` (default: `http://${RELEASE_NAME}-responses-api-store:50051`, used by `scripts/deploy-kind.sh`)
- `BACKGROUND_WORKER_IMAGE_REPO` (default: `duihua-background-worker`)
- `BACKGROUND_WORKER_IMAGE_TAG` (default: `local`)
- `GATEWAY_ROLLOUT_RESTART` (default: `true`, used by `scripts/deploy-kind.sh`)
- `BACKGROUND_WORKER_ROLLOUT_RESTART` (default: `true`, used by `scripts/build-and-load-images.sh` and `scripts/deploy-kind.sh`)
- `INFERENCE_ENABLED` (default: `true`)
- `VALUES_FILE` (default: `charts/duihua-ai-services/values-kind.yaml`)
- `KEDA_NAMESPACE` (default: `keda`)
- `GATEWAY_BASE_URL` (default: `http://127.0.0.1:8080`, used by `scripts/smoke-test-kind.sh`)
- `DEFAULT_MODEL` (default: `HuggingFaceTB/SmolLM2-135M-Instruct`, used by `scripts/smoke-test-kind.sh`)
- `MOCK_VLLM_IMAGE` (default: `ghcr.io/beranekio/mock-vllm:latest`, used by CI mock upstream scripts)

## CI

- **Validate** (`.github/workflows/validate.yml`): Runs on PRs and pushes to `main`. Performs Rust formatting/clippy/tests, Hadolint on the background-worker Dockerfile, `helm dependency build`, `helm lint`, and `helm template` rendering.
- **Kind Integration** (`.github/workflows/kind-integration.yml`): On PRs/pushes that touch the chart, kind assets, scripts, or background worker (manual trigger also available). Creates a kind cluster (`duihua-ci`), runs `scripts/ci-kind-integration.sh` (KEDA, background-worker image build, gateway from the pinned `duihua-gateway` subchart, `ghcr.io/beranekio/mock-vllm:latest` upstream, Helm deploy with `values-kind-ci.yaml`, `scripts/smoke-test-kind.sh` including background completion when the worker Deployment is present).

CI uses the published [mock-vllm](https://github.com/beranekio/mock-vllm) image (`ghcr.io/beranekio/mock-vllm:latest`) instead of the bundled vLLM inference stack. Local kind workflows still use `values-kind.yaml` with real inference via `scripts/kind-local-up.sh`.
