# Builds the renderer demo into demo/dist (index.html + pkg/*.js + pkg/*.wasm).
#
#   pwsh ./build.ps1            # release build
#   pwsh ./build.ps1 -Dev       # debug build (faster to compile, much slower to run)
#   node ./serve.mjs            # then open http://localhost:8080/
#
# The steps are the same for any wasm-bindgen crate:
#   1. cargo build --target wasm32-unknown-unknown
#   2. install the wasm-bindgen CLI at exactly the version of the wasm-bindgen
#      crate in Cargo.lock (a mismatch is a hard error in the CLI)
#   3. wasm-bindgen --target web: emits an ES module plus the .wasm, loaded
#      with WebAssembly.instantiateStreaming, which is allowed by the CSP
#      `script-src 'self' 'wasm-unsafe-eval'`
param([switch]$Dev)

$ErrorActionPreference = 'Stop'

$demo = $PSScriptRoot
$tools = Join-Path (Split-Path $demo) '.tools'    # web/renderer/.tools
$crate = 'webterm_renderer_demo'
$buildProfile = if ($Dev) { 'debug' } else { 'release' }

function Invoke-Checked([string]$what, [scriptblock]$command) {
    & $command
    if ($LASTEXITCODE -ne 0) { throw "$what failed with exit code $LASTEXITCODE" }
}

# 1. Compile. The target directory stays inside the crate (demo/target).
$cargoArgs = @(
    'build',
    '--manifest-path', (Join-Path $demo 'Cargo.toml'),
    '--target', 'wasm32-unknown-unknown'
)
if (-not $Dev) { $cargoArgs += '--release' }
Invoke-Checked 'cargo build' { cargo @cargoArgs }

# 2. wasm-bindgen CLI, pinned to the crate version in the lockfile.
$lock = Get-Content (Join-Path $demo 'Cargo.lock') -Raw
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

# 3. Generate the JS glue next to the static files.
$wasm = Join-Path $demo "target\wasm32-unknown-unknown\$buildProfile\$crate.wasm"
$out = Join-Path $demo 'dist\pkg'
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
Invoke-Checked 'wasm-bindgen' {
    & $bindgen --target web --no-typescript --out-dir $out --out-name $crate $wasm
}

Get-ChildItem $out | ForEach-Object { '{0,10:N0} bytes  {1}' -f $_.Length, $_.FullName }
