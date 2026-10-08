#!/usr/bin/env python3
"""The toolchain pins a Local App runtime is checked against.

Two files make up the effective pins: the shared toolchain pins (Node, pnpm, TypeScript, the
APK identities) are owned by the mobile-linux SDK, and the `local_app_runtime` profile is owned by
this repository. The SDK is a development/CI input handed in by whoever runs the gate (the
product resolves it from its own locked Cargo identity); this repository does not depend on it.
"""
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[2]
PRODUCT_PINS = "docs/runtime/local-app-runtime-pins.json"
SHARED_PINS = "docs/toolchains/runtime-pins.json"


def effective_pins(repo=ROOT, sdk_root=None):
    if not sdk_root:
        raise SystemExit("--sdk-root is required: the shared toolchain pins are owned by the SDK")
    shared = json.loads((Path(sdk_root).resolve(strict=True) / SHARED_PINS).read_text())
    product = json.loads((Path(repo) / PRODUCT_PINS).read_text())
    if set(product) != {"local_app_runtime"}:
        raise ValueError("Local App pins may contain only local_app_runtime; shared pins belong to the SDK")
    return {**shared, **product}
