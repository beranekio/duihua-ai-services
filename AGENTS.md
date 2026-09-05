# AGENTS.md

Guidance for human and AI contributors working in this repository.

## Project overview
- This repo provides a Kubernetes-first, OpenAI API-compatible serving stack.
- Main components:
  - [beranekio/duihua-gateway](https://github.com/beranekio/duihua-gateway) (Rust/Axum API gateway, consumed as Helm subchart)
  - [beranekio/duihua-background-worker](https://github.com/beranekio/duihua-background-worker) (Valkey stream consumer for background Responses API requests, consumed as Helm subchart)
  - `charts/duihua-ai-services` (Helm chart)
  - `scripts/` (local kind + deployment helpers)
  - `docs/` (operations notes)

## Recommended workflow
1. Read `README.md` and relevant files before editing.
2. Keep changes focused and minimal to the requested task.
3. Prefer updating docs/charts/scripts together when behavior changes.
4. Run targeted checks for the area you modified (see [Validation commands](#validation-commands)).
5. Before pushing commits that touch the gateway subchart wiring or Helm chart, run the [kind integration smoke test](#pre-push-kind-integration-test) when your environment supports it.

## Pre-push kind integration test

**Required before push** when a commit changes either:
- `charts/duihua-ai-services/` (chart templates, values, subchart pins, or chart defaults),
- `scripts/` when deploy or smoke-test behavior changes, or
- Gateway or background-worker integration in this repo (values/scripts referencing `duihua-gateway` or `duihua-background-worker`).

Gateway source changes belong in [beranekio/duihua-gateway](https://github.com/beranekio/duihua-gateway); background worker source changes belong in [beranekio/duihua-background-worker](https://github.com/beranekio/duihua-background-worker). Bump the OCI subchart pins in `Chart.yaml` when adopting new published charts.

Run the end-to-end kind smoke test so chart, gateway, and background-worker changes are validated against a real cluster, not only static checks.

### When the environment supports kind

1. Ensure prerequisites are available: `docker`, `kind`, `kubectl`, `helm`, `curl`, and `python3`.
2. From the repository root, use **first-time bootstrap** or **incremental refresh** (not both for the same change).

**First-time bootstrap** (new kind cluster or full reinstall):

```bash
scripts/kind.sh up
scripts/kind.sh smoke
```

Do not re-run `scripts/kind.sh up` to pick up gateway or chart edits on an existing cluster. Use incremental refresh instead.

**Incremental refresh** (cluster already running; required after gateway or chart edits):

```bash
scripts/deploy-kind.sh             # after chart or values changes (restarts Deployments after Helm upgrade)
scripts/smoke-test-kind.sh
```

Gateway and background worker images come from the pinned `duihua-gateway` and `duihua-background-worker` OCI subcharts (GHCR). `scripts/deploy-kind.sh` restarts those Deployments by default after chart upgrades so config changes take effect. Set `GATEWAY_ROLLOUT_RESTART=false` or `BACKGROUND_WORKER_ROLLOUT_RESTART=false` to skip individual restarts.

`scripts/smoke-test-kind.sh` exercises sync and background Responses API flows (including completion polling when the worker Deployment is present), cancel/delete behavior, in-flight continuation rejection, and background-worker Deployment resource requests. See `README.md` (Local kind workflow scripts) for tunables such as `GATEWAY_BASE_URL`, `DEFAULT_MODEL`, `RELEASE_NAME`, and `NAMESPACE`.

### When kind is not available

If Docker, kind, cluster access, or sufficient resources are unavailable, still run the applicable [Validation commands](#validation-commands) below and **state explicitly** in the PR or commit notes that the kind smoke test was not run and why. Do not skip the smoke test silently when the tooling is present.

## Validation commands

Run checks that match the files you changed. Chart and deploy-script edits need both static Helm checks **and** the kind smoke test above when possible. Background-worker Rust changes belong in [beranekio/duihua-background-worker](https://github.com/beranekio/duihua-background-worker).

### Helm chart (`charts/duihua-ai-services`)
- `scripts/update-oci-subcharts.sh` when refreshing OCI subchart pins (or rely on the weekly `Update OCI subcharts` workflow)
- `helm dependency update charts/duihua-ai-services`
- `helm lint charts/duihua-ai-services`
- `helm template duihua charts/duihua-ai-services -f charts/duihua-ai-services/values-kind.yaml >/tmp/duihua-rendered.yaml`

### Scripts
- `bash -n scripts/*.sh` when editing shell helpers

### Kind integration (chart or deploy script changes; see [Pre-push kind integration test](#pre-push-kind-integration-test))
- `scripts/kind.sh up` for first-time bootstrap only
- On an existing cluster: `scripts/kind.sh deploy` or `scripts/deploy-kind.sh` (see [incremental refresh](#when-the-environment-supports-kind))
- `scripts/kind.sh smoke` or `scripts/smoke-test-kind.sh`

For unrelated edits (docs-only, scripts that do not affect deploy behavior, etc.), run only the checks relevant to those paths.

## Editing conventions
- Preserve existing naming and style in each area.
- Avoid unrelated refactors in the same commit.
- Document user-visible changes in `README.md` and/or `docs/operations.md`.
- Keep Kubernetes defaults cloud-provider-neutral unless explicitly required.
- Parent chart values for the gateway use the `duihua-gateway:` key (not `gateway:`). Background worker values use `duihua-background-worker:` (not `backgroundWorker:`).

## Agent-specific notes

### Opening pull requests

When creating a PR, **always add a GitHub label that identifies the agent** (or tooling) that authored it. Use the repo's existing label if one matches; otherwise create it first or ask a maintainer to add it.

| Agent / tool | Label |
| --- | --- |
| ChatGPT Codex | `codex` |
| Cursor | `cursor` |
| Claude | `claude` |
| Grok | `grok` |

Use a short, lowercase slug derived from the agent name when your agent is not listed above.

```bash
# When creating the PR
gh pr create --label grok ...

# Or after the PR exists
gh pr edit --add-label grok
```

Include in the PR description:
- What changed
- Why it changed
- How it was validated (exact commands), including `scripts/smoke-test-kind.sh` when gateway subchart wiring or chart files changed

If a command cannot be run in the current environment, state that clearly (especially the kind smoke test).

If you add new tooling, include brief usage notes in `README.md`.