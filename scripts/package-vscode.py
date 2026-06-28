#!/usr/bin/env python3
"""Build a minimal VSIX package for the SKLint VSCode extension.

The script intentionally avoids a global `vsce` dependency. It compiles the
TypeScript extension only when `vscode/out/extension.js` is missing, builds the
Rust CLI in release mode when needed, and packages the extension with a bundled
platform-native `sklint` executable.
"""

import json
import os
import platform
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[1]
VSCODE_DIR = ROOT / "vscode"
DIST_DIR = ROOT / "dist"

CONTENT_TYPES = """<?xml version=\"1.0\" encoding=\"utf-8\"?>
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">
  <Default Extension=\"json\" ContentType=\"application/json\" />
  <Default Extension=\"js\" ContentType=\"application/javascript\" />
  <Default Extension=\"md\" ContentType=\"text/markdown\" />
  <Default Extension=\"txt\" ContentType=\"text/plain\" />
  <Default Extension=\"xml\" ContentType=\"application/xml\" />
  <Default Extension=\"exe\" ContentType=\"application/octet-stream\" />
  <Override PartName=\"/extension.vsixmanifest\" ContentType=\"text/xml\" />
</Types>
"""


def fail(message: str) -> None:
    raise SystemExit(f"error: {message}")


def find_executable(names: list[str]) -> str | None:
    for name in names:
        found = shutil.which(name)
        if found:
            return found
    return None


def npm_executable() -> str | None:
    # npm is usually npm.cmd on Windows. Checking both names also keeps the
    # error message clear when Node.js is not installed or not exported to PATH.
    if os.name == "nt":
        return find_executable(["npm.cmd", "npm.exe", "npm"])
    return find_executable(["npm"])


def cargo_executable() -> str | None:
    if os.name == "nt":
        return find_executable(["cargo.exe", "cargo"])
    return find_executable(["cargo"])


def run(command: list[str], cwd: Path) -> None:
    print("+", " ".join(command))
    try:
        subprocess.run(command, cwd=cwd, check=True)
    except FileNotFoundError as exc:
        fail(
            f"command was not found: {command[0]}. "
            "Install the required tool or make sure it is available in PATH."
        )


def load_manifest() -> dict:
    manifest_path = VSCODE_DIR / "package.json"
    if not manifest_path.is_file():
        fail(f"VSCode package manifest was not found: {manifest_path}")
    return json.loads(manifest_path.read_text(encoding="utf-8"))


def vsix_manifest(package: dict) -> str:
    categories = ", ".join(package.get("categories", []))
    tags = "python,lint,sklint,static-analysis,formatter"
    engine = package.get("engines", {}).get("vscode", "^1.85.0")
    author = package.get("author", {})
    return f'''<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011">
  <Metadata>
    <Identity Language="en-US" Id="{escape(package["name"])}" Version="{escape(package["version"])}" Publisher="{escape(package["publisher"])}" />
    <DisplayName>{escape(package.get("displayName", package["name"]))}</DisplayName>
    <Description xml:space="preserve">{escape(package.get("description", ""))}</Description>
    <Tags>{escape(tags)}</Tags>
    <Categories>{escape(categories)}</Categories>
    <GalleryFlags>Public</GalleryFlags>
    <Properties>
      <Property Id="Microsoft.VisualStudio.Code.Engine" Value="{escape(engine)}" />
      <Property Id="Microsoft.VisualStudio.Services.Links.Home" Value="{escape(package.get("homepage", ""))}" />
      <Property Id="Microsoft.VisualStudio.Services.Links.Support" Value="mailto:{escape(author.get("email", "stablekite@stablekite.com"))}" />
    </Properties>
  </Metadata>
  <Installation>
    <InstallationTarget Id="Microsoft.VisualStudio.Code" />
  </Installation>
  <Dependencies />
  <Assets>
    <Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" />
  </Assets>
</PackageManifest>
'''


def add_file(archive: zipfile.ZipFile, source: Path, target: str) -> None:
    if source.is_file():
        archive.write(source, target)


def host_platform_tag() -> str:
    system = sys.platform
    machine = platform.machine().lower()
    arch = "x64" if machine in {"amd64", "x86_64"} else machine
    if system.startswith("win"):
        return f"win32-{arch}"
    if system.startswith("linux"):
        return f"linux-{arch}"
    if system == "darwin":
        return f"darwin-{arch}"
    return f"{system}-{arch}"


def host_executable_name() -> str:
    return "sklint.exe" if os.name == "nt" else "sklint"


def release_binary() -> Path:
    return ROOT / "target" / "release" / host_executable_name()


def ensure_extension_compiled() -> None:
    extension_js = VSCODE_DIR / "out" / "extension.js"
    if extension_js.is_file():
        print(f"Using precompiled VSCode extension: {extension_js}")
        return

    npm = npm_executable()
    if not npm:
        fail(
            "npm was not found in PATH and vscode/out/extension.js is missing. "
            "Install Node.js LTS, reopen PowerShell so PATH is refreshed, and run "
            "`npm --version`; or use a release archive that already contains "
            "vscode/out/extension.js."
        )

    if not (VSCODE_DIR / "node_modules" / "typescript").exists():
        run([npm, "install"], VSCODE_DIR)
    run([npm, "run", "compile"], VSCODE_DIR)


def ensure_release_binary() -> Path:
    binary = release_binary()
    if binary.is_file():
        return binary

    cargo = cargo_executable()
    if not cargo:
        fail(
            "cargo was not found in PATH and target/release/sklint was not built. "
            "Install Rust or run this script from a shell where `cargo --version` works."
        )

    run([cargo, "build", "-p", "sklint", "--release"], ROOT)
    if not binary.is_file():
        fail(f"SKLint release binary was not produced: {binary}")
    return binary


def main() -> int:
    package = load_manifest()
    ensure_extension_compiled()
    binary = ensure_release_binary()

    DIST_DIR.mkdir(parents=True, exist_ok=True)
    output = DIST_DIR / f'{package["name"]}-{package["version"]}.vsix'
    if output.exists():
        output.unlink()

    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("extension.vsixmanifest", vsix_manifest(package))
        archive.writestr("[Content_Types].xml", CONTENT_TYPES)
        add_file(archive, VSCODE_DIR / "package.json", "extension/package.json")
        add_file(archive, VSCODE_DIR / "README.ru.md", "extension/README.ru.md")
        add_file(archive, ROOT / "docs" / "rules.ru.md", "extension/docs/rules.ru.md")
        add_file(archive, ROOT / "LICENSE", "extension/LICENSE")
        add_file(archive, VSCODE_DIR / "out" / "extension.js", "extension/out/extension.js")
        add_file(archive, binary, f"extension/bin/{host_platform_tag()}/{host_executable_name()}")

    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
