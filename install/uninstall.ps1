# Removes the native messaging host registration for the current user.
$hostName = "com.webterm.serial"

foreach ($vendor in "Google\Chrome", "Microsoft\Edge") {
    $key = "HKCU:\Software\$vendor\NativeMessagingHosts\$hostName"
    if (Test-Path $key) {
        Remove-Item -Path $key -Recurse -Confirm:$false
    }
}

Write-Host "Unregistered $hostName"
