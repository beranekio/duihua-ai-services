# Operations

## Production recommendations

- Replace permissive CORS in gateway for production.
- Configure API auth at ingress/gateway layer (JWT, API keys, or mTLS).
- Pin explicit runtime image tags (avoid `latest`).
- Add HPA and PodDisruptionBudget based on latency/SLO targets.
- Configure model caching volumes if using larger models.
- KEDA model autoscaling is always active. Ensure KEDA + KEDA HTTP add-on are installed and that traffic is routed through the HTTP interceptor so request-driven scaling works correctly.
- Keep a model warm by setting `inference.autoscaling.default.replicas.min` (or per-model `autoscaling.replicas.min`) to `1` or higher.
- Use an external, production-managed Valkey/Redis-compatible response id store for multi-replica or high-volume gateway deployments if the chart-managed Valkey instance is not sufficient.

## Installing KEDA and KEDA HTTP add-on with Helm

Install once per cluster:

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

Verify core components:

```bash
kubectl get pods -n keda
kubectl get crd | grep -E 'keda.sh|http.keda.sh'
```

Expected CRDs include `scaledobjects.keda.sh` and `interceptorroutes.http.keda.sh`.

## AWS defaults (optional)

If running on AWS EKS:

- Use managed node groups with GPU nodes for inference.
- Install NVIDIA device plugin.
- Optionally use IRSA for pulling models from private S3 buckets.

## Responses API store

The gateway can store completed Responses API objects and materialized conversation input in a Valkey/Redis-compatible store. This lets the gateway serve retrieval, deletion, and input-item requests without waking an inference deployment. For `POST /v1/responses` requests with `previous_response_id`, the gateway loads the saved conversation, appends the previous output and new input, removes `previous_response_id`, and sends a stateless request to the selected upstream model.

Enable the gateway-owned store with `duihua-gateway.responsesApiStore.enabled=true`. This setting does **not** set `VLLM_ENABLE_RESPONSES_API_STORE`: the implementation intentionally avoids vLLM's in-memory response store. Inference deployments may retain the default `autoscaling.replicas.min=0`, scale to zero while idle, and use more than one replica when configured.

The chart-managed Valkey deployment is suitable for development. For production, use an externally managed, highly available Valkey/Redis-compatible service via the `responses-api-store` subchart (`valkey.enabled=false`, external `redis.url`). Tune `duihua-gateway.env.responseIdStoreTtlSeconds` to match how long clients may use stored Responses API ids.

Creation requests that explicitly set `store: false` are not persisted by the gateway. Streaming responses are saved once the gateway observes their `response.completed` event.

### Background Responses API requests

When `duihua-gateway.responsesApiStore.enabled=true` and `duihua-background-worker.enabled=true`, `POST /v1/responses` with `background=true` returns immediately with a `queued` response stored in Valkey and enqueues the `response_id` on a Valkey stream. A `duihua-background-worker` Deployment consumes the stream, issues a synchronous upstream `/responses` call with `background=false` and `store=false`, and updates Valkey when processing completes. Clients poll `GET /v1/responses/{id}` until the status leaves `queued` or `in_progress`. `POST /v1/responses/{id}/cancel` marks the stored response `cancelled` in Valkey (workers must respect terminal statuses).

This path does not change inference Deployments or vLLM flags. Tune `responses-api-store.store.staleSeconds` for stale `in_progress` reconciliation on `GET /v1/responses/{id}` (`duihua-background-worker.staleSeconds` is a deprecated alias; Helm fails at render time when it is set to a value that differs from the subchart setting). Stream retention (trim after `XACK`) is handled by the background-worker consumer, not gateway `XADD`.

#### Background worker autoscaling (optional)

Enable KEDA autoscaling with `duihua-background-worker.autoscaling.enabled=true`. By default (`driver: store-metrics`), the subchart renders a `ScaledObject` that queries the responses-api-store HTTP endpoint `GET /metrics/background-queue?consumer_group=<duihua-background-worker.consumerGroup>` and scales on the JSON `workload` field (`pending + in_progress`). This uses KEDA's `metrics-api` scaler; only KEDA core is required (not the HTTP add-on). KEDA does not connect to Valkey directly. The store derives `workload` from Redis Streams consumer-group stats, so Valkey/Redis **7+** is required for both `store-metrics` and `redis-streams`.

- **Scale-to-zero:** set `duihua-background-worker.autoscaling.replicas.min` to `0`. Keep `activationTargetValue` or `activationLagCount` at `0` so a single queued job activates scaling. Tune `jobsPerReplica` or `lagCount` if brief idle periods flap replicas.
- **Store metrics:** enable `responses-api-store.metrics.enabled=true` (default) so the store Service exposes port `8080` (or `responses-api-store.metrics.port` / `metrics.listenAddr`). When the store subchart is disabled, set `duihua-background-worker.autoscaling.metricsUrl` to a KEDA-reachable URL.
- **In-flight jobs:** `workload` includes claimed but unacknowledged jobs, so scale-down is less aggressive than pure stream lag. Long upstream calls can still be interrupted if `scaledownPeriod` elapses before completion (see issue #52).
- **Scale-to-zero bootstrap:** when autoscaling uses `replicas.min: 0`, the gateway creates the stream consumer group via the responses-api-store gRPC `EnsureConsumerGroup` RPC on startup (workers also ensure the group when they start). KEDA owns Deployment replica counts when autoscaling is enabled; the chart omits `spec.replicas` so Helm upgrades do not reset scaled workers.
- **Warm minimum:** set `replicas.min` to `1` or higher when you want a worker always ready (the kind defaults use `min: 1` so smoke tests do not depend on cold-start latency).
- **Thresholds:** `jobsPerReplica` is the average workload target per replica; lower values scale up sooner. `maxConcurrentJobs` still limits upstream calls per pod.
- **Legacy redis-streams driver:** set `duihua-background-worker.autoscaling.driver: redis-streams` to scale from Valkey consumer-group lag directly. The helper prefers `lagCount` / `activationLagCount` on this driver. With bundled Valkey disabled while the store subchart stays enabled, the scaler still targets the chart-managed Valkey Service name unless issue #58 is addressed; prefer `store-metrics` for external Redis or disable the subchart and set `duihua-gateway.responsesApiStore.redisAddress`.

Local kind (`values-kind.yaml`) enables autoscaling with `min: 1`, `max: 2`, and a low `jobsPerReplica` so `scripts/smoke-test-kind.sh` can verify scale-up when queue workload grows.

#### Local kind and CI scripts

- `scripts/kind.sh` provides `bootstrap`, `up`, `deploy`, `smoke`, and `ci` commands for local kind and GitHub Actions workflows.
- `scripts/deploy-kind.sh` upgrades the Helm release and restarts gateway and background-worker Deployments after chart upgrades via `scripts/restart-deployment.sh`. Gateway and worker images come from the pinned OCI subcharts (GHCR). Kind values enable `serviceAccount.create` so gateway and worker pods use a chart-managed ServiceAccount consistently across upgrades.
- `scripts/smoke-test-kind.sh` waits for the gateway health endpoint and, when background queueing is enabled, for the background-worker Deployment to become ready before exercising completion, cancel, delete, resource checks, and optional KEDA scale-up when `duihua-background-worker.autoscaling.enabled=true`.

Set `duihua-background-worker.enabled=false` to disable the worker Deployment while keeping the response store enabled (leave `duihua-gateway.responsesApiStore.backgroundJobs.enabled=false`, the chart default). Disable `duihua-gateway.responsesApiStore.enabled` to turn off persistence and queueing entirely.

#### Background worker rollouts and graceful shutdown

On SIGTERM (Helm upgrade, `kubectl rollout restart`, or pod deletion), the worker stops issuing new `XREADGROUP` / `XAUTOCLAIM` reads, closes its concurrency limiter, and awaits in-flight upstream jobs before exiting. Tune `duihua-background-worker.terminationGracePeriodSeconds` to exceed `duihua-background-worker.upstreamTimeoutSeconds` plus the configured `duihua-background-worker.blockMs` interval and a safety margin (chart default 665s = 600s upstream + 5s block + 60s margin). SIGTERM can arrive while `XREADGROUP BLOCK` is in flight; the worker lets that read finish before draining any entry it claimed, so grace must cover block time as well as upstream timeout. The worker logs a recommended value at startup; align the chart setting with that figure so rollouts can finish active background Responses API work instead of leaving entries orphaned in the old pod's pending list until autoclaim marks them failed.

`scripts/restart-deployment.sh` with `COMPONENT=background-worker` (used by `deploy-kind.sh`) triggers this path; allow the grace period to elapse before forcing pod deletion.
