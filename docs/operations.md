# Operations

## Production recommendations

- Replace permissive CORS in gateway for production.
- Configure API auth at ingress/gateway layer (JWT, API keys, or mTLS).
- Pin explicit runtime image tags (avoid `latest`).
- Add HPA and PodDisruptionBudget based on latency/SLO targets.
- Configure model caching volumes if using larger models.
- KEDA model autoscaling is always active. Ensure KEDA + KEDA HTTP add-on are installed and that traffic is routed through the HTTP interceptor so request-driven scaling works correctly.
- Keep a model warm by setting `inference.autoscaling.default.replicas.min` (or per-model `autoscaling.replicas.min`) to `1` or higher.

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

Expected CRDs include `scaledobjects.keda.sh` and `httpscaledobjects.http.keda.sh`.

## AWS defaults (optional)

If running on AWS EKS:

- Use managed node groups with GPU nodes for inference.
- Install NVIDIA device plugin.
- Optionally use IRSA for pulling models from private S3 buckets.
