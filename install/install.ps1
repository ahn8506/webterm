# Registers the native messaging host for the current user (Chrome and Edge).
param(
    [string]$ExtensionId = "imhbpbejlppkbemgndcndngolfdoakon",
    [string]$HostExe = (Join-Path $PSScriptRoot "..\host\target\release\webterm-host.exe")
)

$ErrorActionPreference = "Stop"
$hostName = "com.webterm.serial"

if (-not (Test-Path $HostExe)) {
    throw "Host binary not found at $HostExe. Run 'cargo build --release' in the host directory first."
}
$HostExe = (Resolve-Path $HostExe).Path

$manifestPath = Join-Path (Split-Path $HostExe) "$hostName.json"
@{
    name            = $hostName
    description     = "WebTerm serial bridge"
    path            = $HostExe
    type            = "stdio"
    allowed_origins = @("chrome-extension://$ExtensionId/")
} | ConvertTo-Json | Set-Content -Path $manifestPath -Encoding utf8NoBOM

foreach ($vendor in "Google\Chrome", "Microsoft\Edge") {
    $key = "HKCU:\Software\$vendor\NativeMessagingHosts\$hostName"
    New-Item -Path $key -Force | Out-Null
    Set-ItemProperty -Path $key -Name "(default)" -Value $manifestPath
}

Write-Host "Registered $hostName -> $manifestPath"
