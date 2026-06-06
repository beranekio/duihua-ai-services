# AGENTS.md

Guidance for human and AI contributors working in this repository.

## Project overview
- This repo provides a Kubernetes-first, OpenAI API-compatible serving stack.
- Main components:
  - `services/gateway` (Rust/Axum API gateway)
  - `services/background-worker` (lean Rust worker for background Responses API Jobs)
  - `services/common` (shared Rust library for gateway and background worker)
  - `charts/duihua-ai-services` (Helm chart)
  - `scripts/` (local kind + deployment helpers)
  - `docs/` (operations notes)

## Recommended workflow
1. Read `README.md` and relevant files before editing.
2. Keep changes focused and minimal to the requested task.
3. Prefer updating docs/charts/scripts together when behavior changes.
4. Run targeted checks for the area you modified (see [Validation commands](#validation-commands)).
5. Before pushing commits that touch the gateway or Helm chart, run the [kind integration smoke test](#pre-push-kind-integration-test) when your environment supports it.

## Pre-push kind integration test

**Required before push** when a commit changes either:
- `services/gateway/` (gateway source, Dockerfile, or gateway-facing behavior), or
- `charts/duihua-ai-services/` (chart templates, values, or chart defaults).

Run the end-to-end kind smoke test so chart and gateway changes are validated against a real cluster, not only static checks.

### When the environment supports kind

1. Ensure prerequisites are available: `docker`, `kind`, `kubectl`, `helm`, `curl`, and `python3`.
2. From the repository root, use **first-time bootstrap** or **incremental refresh** (not both for the same change).

**First-time bootstrap** (new kind cluster or full reinstall):

```bash
scripts/kind-local-up.sh
scripts/smoke-test-kind.sh
```

Do not re-run `scripts/kind-local-up.sh` to pick up gateway or chart edits on an existing cluster. Use incremental refresh instead.

**Incremental refresh** (cluster already running; required after gateway or chart edits):

```bash
scripts/build-and-load-images.sh   # after gateway image changes (restarts gateway if deployed)
scripts/deploy-kind.sh             # after chart or values changes (restarts gateway after Helm upgrade)
scripts/smoke-test-kind.sh
```

`scripts/build-and-load-images.sh` and `scripts/deploy-kind.sh` restart the gateway deployment by default so pods load a rebuilt `:local` image even when the Helm pod template is unchanged. Set `GATEWAY_ROLLOUT_RESTART=false` to skip restarts, or use a unique `GATEWAY_IMAGE_TAG` instead of `:local`.

`scripts/smoke-test-kind.sh` exercises sync and background Responses API flows, cancel/delete behavior, in-flight continuation rejection, and background Job resource requests against the deployed chart. See `README.md` (Local kind workflow scripts) for tunables such as `GATEWAY_BASE_URL`, `DEFAULT_MODEL`, `RELEASE_NAME`, and `NAMESPACE`.

### When kind is not available

If Docker, kind, cluster access, or sufficient resources are unavailable, still run the applicable [Validation commands](#validation-commands) below and **state explicitly** in the PR or commit notes that the kind smoke test was not run and why. Do not skip the smoke test silently when the tooling is present.

## Validation commands

Run checks that match the files you changed. Gateway and chart edits need both unit/static checks **and** the kind smoke test above when possible.

### Rust workspace (run from `services/`)

The gateway and background worker share a Cargo workspace under `services/`.

```bash
cd services
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Individual crates may be targeted with `-p duihua-gateway`, `-p duihua-background-worker`, or `-p duihua-common`.

### Helm chart (`charts/duihua-ai-services`)
- `helm lint charts/duihua-ai-services`
- `helm template duihua charts/duihua-ai-services >/tmp/duihua-rendered.yaml`

### Scripts
- `bash -n scripts/*.sh` when editing shell helpers

### Kind integration (gateway or chart changes; see [Pre-push kind integration test](#pre-push-kind-integration-test))
- `scripts/kind-local-up.sh` for first-time bootstrap only
- On an existing cluster: `scripts/build-and-load-images.sh` and/or `scripts/deploy-kind.sh` (see [incremental refresh](#when-the-environment-supports-kind))
- `scripts/smoke-test-kind.sh`

For unrelated edits (docs-only, scripts that do not affect deploy behavior, etc.), run only the checks relevant to those paths.

## Editing conventions
- Preserve existing naming and style in each area.
- Avoid unrelated refactors in the same commit.
- Document user-visible changes in `README.md` and/or `docs/operations.md`.
- Keep Kubernetes defaults cloud-provider-neutral unless explicitly required.

## Agent-specific notes

### Opening pull requests

When creating a PR, **always add a GitHub label that identifies the agent** (or tooling) that authored it. Use the repo’s existing label if one matches; otherwise create it first or ask a maintainer to add it.

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
- How it was validated (exact commands), including `scripts/smoke-test-kind.sh` when gateway or chart files changed

If a command cannot be run in the current environment, state that clearly (especially the kind smoke test).

If you add new tooling, include brief usage notes in `README.md`.
