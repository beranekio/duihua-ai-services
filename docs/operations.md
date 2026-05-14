# Operations

## Production recommendations

- Replace permissive CORS in gateway for production.
- Configure API auth at ingress/gateway layer (JWT, API keys, or mTLS).
- Pin explicit runtime image tags (avoid `latest`).
- Add HPA and PodDisruptionBudget based on latency/SLO targets.
- Configure model caching volumes if using larger models.

## AWS defaults (optional)

If running on AWS EKS:

- Use managed node groups with GPU nodes for inference.
- Install NVIDIA device plugin.
- Optionally use IRSA for pulling models from private S3 buckets.

