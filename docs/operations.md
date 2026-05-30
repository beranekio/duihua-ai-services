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

Enable the gateway-owned store with `inference.responsesApiStore.enabled=true`. This setting does **not** set `VLLM_ENABLE_RESPONSES_API_STORE`: the implementation intentionally avoids vLLM's in-memory response store. Inference deployments may retain the default `autoscaling.replicas.min=0`, scale to zero while idle, and use more than one replica when configured.

The chart-managed Valkey deployment is suitable for development. For production, use an externally managed, highly available Valkey/Redis-compatible service by keeping `valkey.enabled=false` and pointing `gateway.env.responseIdStoreUrl` at that service. Tune `gateway.env.responseIdStoreTtlSeconds` to match how long clients may use stored Responses API ids.

Creation requests that explicitly set `store: false` are not persisted by the gateway. Streaming responses are saved once the gateway observes their `response.completed` event. Gateway-owned background execution is optional and disabled by default. Enable it with `gateway.env.responsesApiBackgroundEnabled=true` in Helm or `RESPONSES_API_BACKGROUND_ENABLED=true` when running the gateway directly. Requests with `background: true` require that opt-in setting, the gateway-owned store, and `store: true` (the default). The gateway persists and returns a queued response immediately, performs the vLLM request synchronously in a gateway task with upstream storage disabled, and updates the persisted response for polling through `GET /v1/responses/{response_id}`. `POST /v1/responses/{response_id}/cancel` aborts an in-flight task owned by the current gateway process and persists the cancelled response. Combining `background: true` with `stream: true` is not currently supported. Because in-flight work is owned by a gateway process, avoid restarting or scaling down gateway pods while background responses are queued or in progress.
