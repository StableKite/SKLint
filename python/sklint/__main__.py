import os
import subprocess
import sys
from pathlib import Path


def _packaged_binary() -> Path:
    override = os.environ.get("SKLINT_BINARY")
    if override:
        return Path(override)

    executable_name = "sklint.exe" if os.name == "nt" else "sklint"
    return Path(__file__).resolve().parent / "bin" / executable_name


def main() -> int:
    binary = _packaged_binary()
    if not binary.is_file():
        sys.stderr.write(
            "sklint: packaged Rust binary was not found. "
            "Reinstall the package or set SKLINT_BINARY to a built sklint executable.\n"
        )
        return 2

    argv = [str(binary), *sys.argv[1:]]
    if os.name == "nt":
        completed = subprocess.run(argv)
        return int(completed.returncode)

    os.execv(str(binary), argv)
    raise AssertionError("os.execv returned unexpectedly")


if __name__ == "__main__":
    raise SystemExit(main())
