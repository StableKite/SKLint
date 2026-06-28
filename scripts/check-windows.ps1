$ErrorActionPreference = "Continue"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = Split-Path -Parent $ScriptDir
Set-Location $Root

function Find-Python {
  if (Get-Command py -ErrorAction SilentlyContinue) {
    return @{ Exe = "py"; Args = @("-3") }
  }
  if (Get-Command python -ErrorAction SilentlyContinue) {
    return @{ Exe = "python"; Args = @() }
  }
  return $null
}

Write-Host "`n== Project identity =="
Get-Content .\README.md -TotalCount 12

Write-Host "`n== Windows Rust checks =="
cargo --version
rustc --version
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo build -p sklint

$WinBin = Join-Path $Root "target\debug\sklint.exe"
if (-not (Test-Path $WinBin)) { $WinBin = Join-Path $Root "target\debug\sklint" }

Write-Host "`n== Windows CLI smoke =="
& $WinBin --version
& $WinBin check examples\good.py
Write-Host "good.py ExitCode=$LASTEXITCODE ; 0 is expected"
& $WinBin check examples
Write-Host "examples ExitCode=$LASTEXITCODE ; 1 is expected"
& $WinBin check --format json examples\bad.py
Write-Host "bad.py JSON ExitCode=$LASTEXITCODE ; 1 is expected"
& $WinBin check --format json examples\bad.pyi
Write-Host "bad.pyi JSON ExitCode=$LASTEXITCODE ; 1 is expected"
Get-Content examples\bad.py -Raw | & $WinBin check --format json --stdin-filename examples\bad.py -
Write-Host "stdin JSON ExitCode=$LASTEXITCODE ; 1 is expected"
& $WinBin format --check examples\bad.py
Write-Host "format --check ExitCode=$LASTEXITCODE ; 1 is expected"
Copy-Item examples\bad.py "$env:TEMP\sklint-format-smoke.py" -Force
& $WinBin format "$env:TEMP\sklint-format-smoke.py"
& $WinBin check "$env:TEMP\sklint-format-smoke.py"
Write-Host "formatted temp check ExitCode=$LASTEXITCODE ; 1 is expected because human semantic issues can remain"

Write-Host "`n== Windows Python package smoke =="
$Py = Find-Python
if ($Py) {
  $PyPkgSmoke = Join-Path $env:TEMP "sklint-python-package-smoke-win"
  if (Test-Path $PyPkgSmoke) { Remove-Item $PyPkgSmoke -Recurse -Force }
  New-Item -ItemType Directory -Path $PyPkgSmoke -Force | Out-Null
  $WheelDir = Join-Path $PyPkgSmoke "wheelhouse"
  New-Item -ItemType Directory -Path $WheelDir -Force | Out-Null
  $VenvDir = Join-Path $PyPkgSmoke "venv"
  & $Py.Exe @($Py.Args) -m venv "$VenvDir"
  $VenvPython = Join-Path $VenvDir "Scripts\python.exe"
  $VenvSklint = Join-Path $VenvDir "Scripts\sklint.exe"
  if (Test-Path $VenvPython) {
    & $VenvPython -m pip wheel "$Root" -w "$WheelDir" --no-deps
    if ($LASTEXITCODE -ne 0) {
      Write-Host "pip wheel with build isolation failed; retrying with --no-build-isolation"
      & $VenvPython -m pip wheel "$Root" -w "$WheelDir" --no-deps --no-build-isolation
    }
    Write-Host "pip wheel ExitCode=$LASTEXITCODE ; 0 is expected"
    $Wheel = Get-ChildItem $WheelDir -Filter "sklint-*.whl" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($Wheel) {
      Write-Host "Built wheel: $($Wheel.Name)"
      & $VenvPython -m pip install --force-reinstall "$($Wheel.FullName)"
      Write-Host "pip install wheel ExitCode=$LASTEXITCODE ; 0 is expected"
      & $VenvSklint --version
      Write-Host "installed sklint --version ExitCode=$LASTEXITCODE ; 0 is expected"
      & $VenvPython -m sklint --version
      Write-Host "python -m sklint --version ExitCode=$LASTEXITCODE ; 0 is expected"
      & $VenvSklint check examples\good.py
      Write-Host "installed sklint good.py ExitCode=$LASTEXITCODE ; 0 is expected"
    } else {
      Write-Host "No sklint wheel was produced."
    }
  }
} else {
  Write-Host "Python was not found; skipping Python package smoke."
}

Write-Host "`n== VSCode extension build =="
$Npm = Get-Command npm -ErrorAction SilentlyContinue
$PrecompiledExtension = Join-Path $Root "vscode\out\extension.js"
if ($Npm) {
  Push-Location vscode
  npm install
  npm run compile
  Pop-Location
} elseif (Test-Path $PrecompiledExtension) {
  Write-Host "npm was not found; using precompiled vscode\out\extension.js from the archive."
} else {
  Write-Host "npm was not found and vscode\out\extension.js is missing. Install Node.js LTS or use a release archive with precompiled VSCode output."
}
if ($Py) {
  & $Py.Exe @($Py.Args) scripts\package-vscode.py
  Write-Host "VSIX package ExitCode=$LASTEXITCODE ; 0 is expected"
}
$Vsix = Get-ChildItem (Join-Path $Root "dist") -Filter "sklint-*.vsix" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($Vsix) { Write-Host "Built VSIX: $($Vsix.FullName)" }

Write-Host "`n== Pyright strict probe for SK701/SK702 boundary =="
$PyrightProbe = Join-Path $env:TEMP "sklint-pyright-probe"
if (Test-Path $PyrightProbe) { Remove-Item $PyrightProbe -Recurse -Force }
New-Item -ItemType Directory -Path $PyrightProbe | Out-Null
@'
class Box:
    def attach(self) -> None:
        self.generated = 1

    def read(self) -> int:
        return self.generated
'@ | Set-Content -Encoding UTF8 (Join-Path $PyrightProbe "pyright_clean_self_dynamic.py")
@'
class Box:
    pass

box = Box()
box.generated = 1
print(box.generated)
'@ | Set-Content -Encoding UTF8 (Join-Path $PyrightProbe "pyright_reports_object_dynamic.py")
@'
from typing import Any

class DataDict:
    def __getattr__(self, name: str) -> Any:
        return None

    def __setattr__(self, name: str, value: Any) -> None:
        pass

class Common(DataDict):
    pass

common = Common()
common.camera_config = object()
'@ | Set-Content -Encoding UTF8 (Join-Path $PyrightProbe "pyright_clean_dynamic_container.py")

@'
{ "typeCheckingMode": "strict" }
'@ | Set-Content -Encoding UTF8 (Join-Path $PyrightProbe "pyrightconfig.json")
if (Get-Command npx -ErrorAction SilentlyContinue) {
  Push-Location $PyrightProbe
  npx pyright --version
  npx pyright pyright_clean_self_dynamic.py
  Write-Host "pyright_clean_self_dynamic.py ExitCode=$LASTEXITCODE ; 0 is expected"
  npx pyright pyright_clean_dynamic_container.py
  Write-Host "pyright_clean_dynamic_container.py ExitCode=$LASTEXITCODE ; 0 is expected"
  npx pyright pyright_reports_object_dynamic.py
  Write-Host "pyright_reports_object_dynamic.py ExitCode=$LASTEXITCODE ; 1 is expected"
  Pop-Location
} else {
  Write-Host "npx was not found; skipping Pyright probe."
}

Write-Host "`n== WSL CLI smoke =="
$WslAvailable = $false
$WslRoot = ""
$Distro = ""
function ConvertTo-WslPath([string]$WindowsPath) {
  $Normalized = $WindowsPath -replace "\\", "/"
  return (wsl.exe wslpath -a "$Normalized").Trim()
}
if (Get-Command wsl.exe -ErrorAction SilentlyContinue) {
  $Distro = ((wsl.exe -l -q) -replace "`0", "" | Where-Object { $_.Trim().Length -gt 0 } | Select-Object -First 1).Trim()
  if ($Distro) {
    $WslAvailable = $true
    $WslRoot = ConvertTo-WslPath $Root
    $WslScript = @"
set -o pipefail
cd '$WslRoot'
cargo --version
rustc --version
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo build -p sklint
./target/debug/sklint --version
./target/debug/sklint check examples/good.py; echo WSL_GOOD_EXIT=`$?
./target/debug/sklint check examples; echo WSL_EXAMPLES_EXIT=`$?
./target/debug/sklint check --format json examples/bad.pyi; echo WSL_PYI_EXIT=`$?
cat examples/bad.py | ./target/debug/sklint check --format json --stdin-filename examples/bad.py -; echo WSL_STDIN_EXIT=`$?
./target/debug/sklint format --check examples/bad.py; echo WSL_FORMAT_CHECK_EXIT=`$?
"@
    wsl.exe -d $Distro -- bash -lc $WslScript
  } else {
    Write-Host "WSL is installed, but no distro was found."
  }
} else {
  Write-Host "wsl.exe was not found."
}

Write-Host "`n== Install VSIX and open VSCode windows =="
$CodeCmd = $null
if (Get-Command code.cmd -ErrorAction SilentlyContinue) { $CodeCmd = "code.cmd" }
elseif (Get-Command code -ErrorAction SilentlyContinue) { $CodeCmd = "code" }

$ExtensionId = "stablekite.sklint"
$OldNodeOptions = $env:NODE_OPTIONS
if ($env:NODE_OPTIONS) {
  if ($env:NODE_OPTIONS -notmatch "--no-deprecation") {
    $env:NODE_OPTIONS = "$($env:NODE_OPTIONS) --no-deprecation"
  }
} else {
  $env:NODE_OPTIONS = "--no-deprecation"
}

if ($CodeCmd -and $Vsix) {
  & $CodeCmd --install-extension "$($Vsix.FullName)" --force
  Write-Host "Windows VSIX install ExitCode=$LASTEXITCODE ; 0 is expected"
  $WinInstalled = (& $CodeCmd --list-extensions) -contains $ExtensionId
  Write-Host "Windows VSCode has $ExtensionId installed: $WinInstalled"

  if ($WslAvailable) {
    $RemoteAuthority = "wsl+$Distro"
    & $CodeCmd --remote $RemoteAuthority --install-extension "$($Vsix.FullName)" --force
    Write-Host "WSL VSIX install ExitCode=$LASTEXITCODE ; 0 is expected"
    $RemoteInstalled = (& $CodeCmd --remote $RemoteAuthority --list-extensions) -contains $ExtensionId
    Write-Host "WSL VSCode has $ExtensionId installed: $RemoteInstalled"
  }

  & $CodeCmd --new-window "$Root" "$Root\examples\bad.py"
  if ($WslAvailable) {
    $WslFile = "$WslRoot/examples/bad.py"
    & $CodeCmd --folder-uri "vscode-remote://wsl+$Distro$WslRoot" --goto "vscode-remote://wsl+$Distro$WslFile:1:1"
  }
} else {
  Write-Host "VSCode CLI or VSIX was not found; skipping VSCode install/open checks."
}
$env:NODE_OPTIONS = $OldNodeOptions

Write-Host "`nOpened examples\bad.py. Check Problems, native hover diagnostics, noqa hover help, quick fixes, formatter and suppressions."
