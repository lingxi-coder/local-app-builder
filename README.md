# local-app

The Local App project: letting an agent create, build, run and modify small local web apps from a plan the owner has
approved, and the plugin (`create-local-app` and its skills) that exposes it. The working name is `local-app`.

It was developed inside `harness-runtime` (LingXi's engine) and extracted from it. It must not depend on that repository or
on any product repository: `scripts/checks/check-deps.sh` holds the graph to that, and `check-contract-digests.sh` pins the
template catalogue.

**Status.** Extracted from `harness-runtime`, branch `local-app/p2-decouple` at `8f15ed5`, with the history of the files that moved. Not
published: there is no remote. **The licence is undecided.** `license` in the manifests is carried over from the source
workspace and is not a decision, and there is deliberately no LICENSE file; decide before anything is made public.

## Layout

| path | what it is |
| --- | --- |
| `crates/local-app-contracts` | ids and the vocabulary shared by the service and its hosts; no dependencies |
| `crates/local-apps` | the core: app state, persistence (SQLite), checkpoints, authoring/QA and MCP-authoring rules, the plugin packer |
| `crates/local-app-service` | orchestration: the broker, the build pipeline, the in-process MCP server, and the seams a host implements |
| `crates/device-api` | device capability traits |
| `crates/mcp-wire` | MCP wire types and the in-process transport traits |
| `crates/rooted-fs` | rooted file operations: containment, atomic writes, file locks |
| `crates/local-app-plugin` | where the plugin tree is, and the schemas, workflow scripts and fixtures a host embeds |
| `crates/plugins/lingxi-local-app` | the plugin tree: skills, agents, workflows, schemas, template assets (`.inventory.txt` beside it lists its files) |
| `docs/local-apps` | design history (Chinese) and the performance baselines the core embeds |

## Build and test

    cargo test --workspace
    ./scripts/check-all.sh

`git2` is requested from crates.io with default features off (libgit2 only for local repositories). A workspace that already
links libgit2 chooses the copy with its own `[patch.crates-io]`: one package may link it. Without the `git-checkpoints`
feature of `local-apps` (and of `local-app-service`) the graph has no libgit2 at all.

## Used by

`harness-runtime` (LingXi's engine) and, through it, the LingXi mobile app. Both must pin this repository at the same
revision. Desktop agents (Codex, Claude Code) are the next consumers.
