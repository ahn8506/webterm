# Builds the terminal WASM module into extension/wasm.
# Same steps as web/renderer/demo/build.ps1, and it shares that wasm-bindgen CLI.
param([switch]$Dev)

$ErrorActionPreference = 'Stop'

$app = $PSScriptRoot
$tools = Join-Path $app '..\renderer\.tools'
$out = Join-Path $app '..\..\extension\wasm'
$crate = 'webterm_app'
$buildProfile = if ($Dev) { 'debug' } else { 'release' }

function Invoke-Checked([string]$what, [scriptblock]$command) {
    & $command
    if ($LASTEXITCODE -ne 0) { throw "$what failed with exit code $LASTEXITCODE" }
}

$cargoArgs = @(
    'build',
    '--manifest-path', (Join-Path $app 'Cargo.toml'),
    '--target', 'wasm32-unknown-unknown'
)
if (-not $Dev) { $cargoArgs += '--release' }
Invoke-Checked 'cargo build' { cargo @cargoArgs }

# The CLI must match the wasm-bindgen crate version in the lockfile exactly.
$lock = Get-Content (Join-Path $app 'Cargo.lock') -Raw
if ($lock -notmatch '(?m)^name = "wasm-bindgen"\r?\nversion = "([^"]+)"') {
    throw 'wasm-bindgen not found in Cargo.lock'
}
$wanted = $Matches[1]
$bindgen = Join-Path $tools 'bin\wasm-bindgen.exe'
$installed = if (Test-Path $bindgen) { (& $bindgen --version) -replace '^wasm-bindgen\s+', '' } else { '' }
if ($installed -ne $wanted) {
    Write-Host "installing wasm-bindgen-cli $wanted into $tools (found: '$installed')"
    Invoke-Checked 'cargo install wasm-bindgen-cli' {
        cargo install wasm-bindgen-cli --version $wanted --locked --root $tools
    }
}

$wasm = Join-Path $app "target\wasm32-unknown-unknown\$buildProfile\$crate.wasm"
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
Invoke-Checked 'wasm-bindgen' {
    & $bindgen --target web --no-typescript --out-dir $out --out-name $crate $wasm
}

Get-ChildItem $out | ForEach-Object { '{0,10:N0} bytes  {1}' -f $_.Length, $_.FullName }
