import os
import shutil
import stat
import subprocess
import sys
from pathlib import Path
from typing import Optional, Tuple

from setuptools import setup
from setuptools.command.build_py import build_py as _build_py

try:
    from wheel.bdist_wheel import bdist_wheel as _bdist_wheel
except Exception:  # pragma: no cover - wheel is declared as a build dependency.
    _bdist_wheel = None


ROOT = Path(__file__).parent.resolve()


def _target_triple_from_rustc(cargo: str) -> Optional[str]:
    rustc = os.environ.get("RUSTC") or shutil.which("rustc")
    command = [rustc, "-vV"] if rustc else [cargo, "rustc", "-p", "sklint", "--", "-vV"]
    try:
        output = subprocess.check_output(command, cwd=ROOT, text=True, stderr=subprocess.STDOUT)
    except (OSError, subprocess.CalledProcessError):
        return None
    for line in output.splitlines():
        if line.startswith("host: "):
            return line.split(": ", 1)[1].strip()
    return None


class build_py(_build_py):
    """Build the Rust CLI and package it inside the Python wheel."""

    def run(self) -> None:
        super().run()
        self._build_and_copy_rust_binary()

    def _build_and_copy_rust_binary(self) -> None:
        cargo = os.environ.get("CARGO", "cargo")
        profile = os.environ.get("SKLINT_CARGO_PROFILE", "release").strip().lower()
        if profile not in {"debug", "release"}:
            raise RuntimeError("SKLINT_CARGO_PROFILE must be either 'debug' or 'release'")

        command = [cargo, "build", "-p", "sklint"]
        if profile == "release":
            command.append("--release")

        subprocess.run(command, cwd=ROOT, check=True)

        executable_name = "sklint.exe" if os.name == "nt" else "sklint"
        source = ROOT / "target" / profile / executable_name
        if not source.is_file():
            raise RuntimeError(f"Rust build succeeded, but binary was not found: {source}")

        destination = Path(self.build_lib) / "sklint" / "bin" / executable_name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)

        if os.name != "nt":
            mode = destination.stat().st_mode
            destination.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

        target = _target_triple_from_rustc(cargo)
        if target:
            marker = Path(self.build_lib) / "sklint" / "bin" / "TARGET"
            marker.write_text(f"{target}\n", encoding="utf-8")


cmdclass = {"build_py": build_py}

if _bdist_wheel is not None:
    class bdist_wheel(_bdist_wheel):  # type: ignore[misc, valid-type]
        """Mark the wheel as platform-specific because it contains a native binary."""

        def finalize_options(self) -> None:
            super().finalize_options()
            self.root_is_pure = False

        def get_tag(self) -> Tuple[str, str, str]:
            _python_tag, _abi_tag, platform_tag = super().get_tag()
            return "py3", "none", platform_tag

    cmdclass["bdist_wheel"] = bdist_wheel


setup(cmdclass=cmdclass)
