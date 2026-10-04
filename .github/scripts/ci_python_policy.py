from __future__ import annotations

import argparse
import platform
import sys
import sysconfig


def _call_bool(obj: object, name: str) -> bool:
    func = getattr(obj, name, None)
    if not callable(func):
        return False
    try:
        return bool(func())
    except Exception:
        return False


def verify() -> None:
    is_cpython = platform.python_implementation() == "CPython"
    stable = sys.version_info.releaselevel == "final"
    free_threaded = bool(sysconfig.get_config_var("Py_GIL_DISABLED"))
    version_ok = sys.version_info >= (3, 14)

    jit = getattr(sys, "_jit", None)
    jit_available = _call_bool(jit, "is_available") if jit is not None else False
    jit_enabled = _call_bool(jit, "is_enabled") if jit is not None else False

    print(
        f"CPython {sys.version_info.major}.{sys.version_info.minor} "
        f"cpython={is_cpython} stable={stable} free_threaded={free_threaded} "
        f"jit_available={jit_available} jit_enabled={jit_enabled}"
    )

    if not is_cpython:
        raise SystemExit("CI Python policy failure: CPython is required")
    if not stable:
        raise SystemExit("CI Python policy failure: prerelease Python is forbidden")
    if not version_ok:
        raise SystemExit("CI Python policy failure: free-threaded CPython >=3.14 is required")
    if not free_threaded:
        raise SystemExit("CI Python policy failure: GIL-enabled build is forbidden")
    if jit_available and not jit_enabled:
        raise SystemExit(
            "CI Python policy failure: selected build supports JIT but JIT is not enabled"
        )

    if jit_available:
        print("CI Python JIT mode: enabled")
    else:
        print("CI Python JIT mode: unavailable in selected build; latest stable t retained")
    print("CI Python runtime policy: PASS")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    if not args.verify:
        parser.error("--verify is required")
    verify()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
