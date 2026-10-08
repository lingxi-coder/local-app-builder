# local-app-builder

The Local App project: letting an agent create, build, run and modify small local web apps from a plan the owner has
approved, and the plugin (`create-local-app` and its skills) that exposes it. The working name is `local-app-builder`.

It was developed inside `harness-runtime` (LingXi's engine) and extracted from it. It must not depend on that repository or
on any product repository: `scripts/checks/check-deps.sh` holds the graph to that, and `check-contract-digests.sh` pins the
template catalogue.

**Status.** Extracted from `harness-runtime`, branch `local-app/p2-decouple` at `8f15ed5`, with the history of the files that moved. The
repository is public (`lingxi-coder/local-app-builder`) and licensed under the MIT License (see `LICENSE`).

## Layout

| path | what it is |
| --- | --- |
| `crates/local-app-builder-contracts` | ids and the vocabulary shared by the service and its hosts; no dependencies |
| `crates/local-apps` | the core: app state, persistence (SQLite), checkpoints, authoring/QA and MCP-authoring rules, the plugin packer |
| `crates/local-app-builder-service` | orchestration: the broker, the build pipeline, the in-process MCP server, and the seams a host implements |
| `crates/local-app-builder-cli` | the `local-app-builder` command (a leaf; nothing depends on it): `version`, `doctor`, `toolchain` and the stdio MCP server (`mcp`) |
| `crates/device-api` | device capability traits |
| `crates/mcp-wire` | MCP wire types and the in-process transport traits |
| `crates/rooted-fs` | rooted file operations: containment, atomic writes, file locks |
| `crates/local-app-builder-plugin` | where the plugin tree is, and the schemas, workflow scripts and fixtures a host embeds |
| `crates/plugins/lingxi-local-app` | the plugin tree: skills, agents, workflows, schemas, template assets (`.inventory.txt` beside it lists its files) |
| `docs/local-apps` | design history (Chinese) and the performance baselines the core embeds |
| `docs/runtime` | the runtime seed's pins, policy and SBOM (`local-app-runtime-{pins,policy}.json`, `sbom/`) |
| `scripts/runtime` | build and attest the runtime seed: `verify-local-app-supply-chain.py`, `stage-local-app-runtime.py`, `build-local-app-node-modules.py`, `generate-local-app-sbom.py` |

## Build and test

    cargo test --workspace
    ./scripts/check-all.sh

`git2` is requested from crates.io with default features off (libgit2 only for local repositories). A workspace that already
links libgit2 chooses the copy with its own `[patch.crates-io]`: one package may link it. Without the `git-checkpoints`
feature of `local-apps` (and of `local-app-builder-service`) the graph has no libgit2 at all.

### The runtime seed

The templates under `crates/local-apps/templates/runtime-profiles` are what a host stages onto a device. The scripts in
`scripts/runtime` check them against the pins in `docs/runtime`, and stage them read-only for a native app bundle. The shared
toolchain pins (Node, pnpm, TypeScript, the APK identities) belong to the mobile-linux SDK, which this repository does not depend on,
so every script takes `--sdk-root <checkout>`. `scripts/tests/test-local-app-supply-chain.sh --sdk-root <checkout>` proves the
verifier rejects what it should; it needs that checkout too, so it runs in the product's CI rather than here.

## Used by

`harness-runtime` (LingXi's engine) and, through it, the LingXi mobile app. Both must pin this repository at the same
revision. Desktop agents (Codex, Claude Code) are the next consumers.
