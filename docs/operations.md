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

Enable the gateway-owned store with `gateway.responsesApiStore.enabled=true`. This setting does **not** set `VLLM_ENABLE_RESPONSES_API_STORE`: the implementation intentionally avoids vLLM's in-memory response store. Inference deployments may retain the default `autoscaling.replicas.min=0`, scale to zero while idle, and use more than one replica when configured.

The chart-managed Valkey deployment is suitable for development. For production, use an externally managed, highly available Valkey/Redis-compatible service by keeping `valkey.enabled=false` and pointing `gateway.env.responseIdStoreUrl` at that service. Tune `gateway.env.responseIdStoreTtlSeconds` to match how long clients may use stored Responses API ids.

Creation requests that explicitly set `store: false` are not persisted by the gateway. Streaming responses are saved once the gateway observes their `response.completed` event.

### Background Responses API requests

When `gateway.responsesApiStore.enabled=true` and `backgroundWorker.enabled=true` (default), `POST /v1/responses` with `background=true` returns immediately with a `queued` response stored in Valkey and enqueues the `response_id` on a Valkey stream. A `duihua-background-worker` Deployment consumes the stream, issues a synchronous upstream `/responses` call with `background=false` and `store=false`, and updates Valkey when processing completes. Clients poll `GET /v1/responses/{id}` until the status leaves `queued` or `in_progress`. `POST /v1/responses/{id}/cancel` marks the stored response `cancelled` in Valkey (workers must respect terminal statuses).

This path does not change inference Deployments or vLLM flags. Tune `backgroundWorker.streamKey` and `backgroundWorker.staleSeconds` for queue routing and stale-response reconciliation on `GET /v1/responses/{id}`. Stream retention (trim after `XACK`) is handled by the background-worker consumer, not gateway `XADD`.

#### Background worker autoscaling (optional)

Enable KEDA stream-lag autoscaling with `backgroundWorker.autoscaling.enabled=true`. The chart renders a `ScaledObject` that scales the worker Deployment from consumer-group lag on the background queue stream (`backgroundWorker.streamKey`, group `backgroundWorker.consumerGroup`). This uses KEDA's `redis-streams` scaler with `lagCount` and `activationLagCount`; only KEDA core is required (not the HTTP add-on).

- **Scale-to-zero:** set `backgroundWorker.autoscaling.replicas.min` to `0`. Requires Valkey/Redis 7+ for lag-based scaling (the chart's Valkey 9.x image qualifies). Keep `activationLagCount` at `0` so a single queued job activates scaling (KEDA compares lag with `>`). Tune `lagCount` if brief idle periods flap replicas.
- **External Redis:** the ScaledObject parses `gateway.env.responseIdStoreUrl` with Helm `urlParse` for `host:port`, enables TLS when the scheme is `rediss://`, and can set `passwordFromEnv` to an env var present on worker pods.
- **In-flight jobs:** lag drops once a message is claimed; long upstream calls can be interrupted if `scaledownPeriod` elapses before completion (see issue #52).
- **Scale-to-zero bootstrap:** when autoscaling uses `replicas.min: 0`, a Helm hook Job creates the stream consumer group before workers scale from zero (the group is otherwise created on first worker startup). The hook fails on connectivity or auth errors and only ignores an already-existing group (`BUSYGROUP`). Configure `backgroundWorker.queueBootstrap.image` for external Redis when the default `redis:7-alpine` image is unavailable. KEDA owns Deployment replica counts when autoscaling is enabled; the chart omits `spec.replicas` so Helm upgrades do not reset scaled workers.
- **Warm minimum:** set `replicas.min` to `1` or higher when you want a worker always ready (the kind defaults use `min: 1` so smoke tests do not depend on cold-start latency).
- **Thresholds:** `lagCount` is the average lag target per replica; lower values scale up sooner. `maxConcurrentJobs` still limits upstream calls per pod.
- **Redis address:** when using the chart-managed Valkey Service, the ScaledObject uses a cluster DNS FQDN (`<release>-valkey.<namespace>.svc.cluster.local`) so KEDA (typically in the `keda` namespace) can reach Redis. For an external store, set `gateway.env.responseIdStoreUrl` to a hostname KEDA can resolve cluster-wide.

Local kind (`values-kind.yaml`) enables autoscaling with `min: 1`, `max: 2`, and a low `lagCount` so `scripts/smoke-test-kind.sh` can verify scale-up when queue lag grows.

#### Local kind and CI scripts

- `scripts/build-and-load-images.sh` builds and loads gateway and background-worker images, then restarts both Deployments when deployed.
- `scripts/deploy-kind.sh` upgrades the Helm release and restarts gateway and background-worker Deployments so `:local` image tags are picked up. Kind values enable `serviceAccount.create` so gateway and worker pods use a chart-managed ServiceAccount consistently across upgrades.
- `scripts/restart-background-worker-deployment.sh` restarts only the worker Deployment (optional `BACKGROUND_WORKER_DEPLOYMENT_REQUIRED=true` for hard failure when missing).
- `scripts/smoke-test-kind.sh` waits for the gateway health endpoint and, when background queueing is enabled, for the background-worker Deployment to become ready before exercising completion, cancel, delete, resource checks, and optional KEDA scale-up when `backgroundWorker.autoscaling.enabled=true`.

Set `backgroundWorker.enabled=false` to disable the worker Deployment while keeping the response store enabled. Disable `gateway.responsesApiStore.enabled` to turn off persistence and queueing entirely.
