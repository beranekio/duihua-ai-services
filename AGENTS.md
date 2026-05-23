# AGENTS.md

Guidance for human and AI contributors working in this repository.

## Project overview
- This repo provides a Kubernetes-first, OpenAI API-compatible serving stack.
- Main components:
  - `services/gateway` (Rust/Axum API gateway)
  - `charts/duihua-ai-services` (Helm chart)
  - `scripts/` (local kind + deployment helpers)
  - `docs/` (operations notes)

## Recommended workflow
1. Read `README.md` and relevant files before editing.
2. Keep changes focused and minimal to the requested task.
3. Prefer updating docs/charts/scripts together when behavior changes.
4. Run targeted checks for the area you modified.

## Validation commands
- Rust gateway:
  - `cargo fmt --all --check` (from `services/gateway`)
  - `cargo clippy --all-targets --all-features -- -D warnings` (from `services/gateway`)
  - `cargo test` (from `services/gateway`)
- Helm chart:
  - `helm lint charts/duihua-ai-services`
  - `helm template duihua charts/duihua-ai-services >/tmp/duihua-rendered.yaml`
- Optional script sanity checks:
  - `bash -n scripts/*.sh`

Run only the checks needed for files you touched; run broader checks before large PRs.

## Editing conventions
- Preserve existing naming and style in each area.
- Avoid unrelated refactors in the same commit.
- Document user-visible changes in `README.md` and/or `docs/operations.md`.
- Keep Kubernetes defaults cloud-provider-neutral unless explicitly required.

## Agent-specific notes
- Before opening a PR, include:
  - What changed
  - Why it changed
  - How it was validated (exact commands)
- If a command cannot be run in the current environment, state that clearly.
- If you add new tooling, include brief usage notes in `README.md`.
