# Installs SudoVDA from spike\driver\pkg the way Apollo's install.bat does:
# trust the package's self-signed certificate (Root + TrustedPublisher), recreate the root device node,
# install the INF. Then checks the device came up. Run fetch.ps1 first. MUST run elevated.
#   powershell -ExecutionPolicy Bypass -File spike\driver\install.ps1
# Undo with uninstall.ps1.
$ErrorActionPreference = 'Stop'
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) {
    Write-Host 'install.ps1 must run elevated: open PowerShell with "Run as administrator" and run it again.' -ForegroundColor Red
    exit 1
}

$pkg = Join-Path $PSScriptRoot 'pkg'
foreach ($f in 'SudoVDA.dll', 'SudoVDA.inf', 'sudovda.cat', 'sudovda.cer', 'nefconc.exe') {
    if (-not (Test-Path (Join-Path $pkg $f))) { throw "pkg\$f is missing. Run fetch.ps1 first." }
}
$hwid = 'root\sudomaker\sudovda'
$classGuid = '4D36E968-E325-11CE-BFC1-08002BE10318'
$nefconc = Join-Path $pkg 'nefconc.exe'

try { Write-Host "Secure Boot on: $(Confirm-SecureBootUEFI)" } catch { Write-Host "Secure Boot state unknown: $($_.Exception.Message)" }

$cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 (Join-Path $pkg 'sudovda.cer')
Write-Host "Trusting $($cert.Subject) (thumbprint $($cert.Thumbprint)) in LocalMachine Root and TrustedPublisher"
certutil -addstore -f root (Join-Path $pkg 'sudovda.cer') | Out-Null
if ($LASTEXITCODE -ne 0) { throw "certutil -addstore root failed ($LASTEXITCODE)" }
certutil -addstore -f TrustedPublisher (Join-Path $pkg 'sudovda.cer') | Out-Null
if ($LASTEXITCODE -ne 0) { throw "certutil -addstore TrustedPublisher failed ($LASTEXITCODE)" }

Write-Host 'Removing any old device node (an error here is normal on a first install)'
& $nefconc --remove-device-node --hardware-id $hwid --class-guid $classGuid
Write-Host 'Creating the device node'
& $nefconc --create-device-node --class-name Display --class-guid $classGuid --hardware-id $hwid
if ($LASTEXITCODE -ne 0) { throw "nefconc --create-device-node failed ($LASTEXITCODE)" }
Write-Host 'Installing the driver'
& $nefconc --install-driver --inf-path (Join-Path $pkg 'SudoVDA.inf')
if ($LASTEXITCODE -ne 0) { throw "nefconc --install-driver failed ($LASTEXITCODE)" }

Write-Host 'Waiting for the device...'
$dev = $null
for ($i = 0; $i -lt 20 -and -not ($dev -and $dev.Status -eq 'OK'); $i++) {
    Start-Sleep -Milliseconds 500
    $dev = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.HardwareID -contains $hwid } | Select-Object -First 1
}
if ($dev -and $dev.Status -eq 'OK') {
    Write-Host "SudoVDA is up: $($dev.FriendlyName) [$($dev.InstanceId)] status $($dev.Status)" -ForegroundColor Green
    Write-Host 'Next (not elevated): spike\target\release\cropcast-spike.exe'
} else {
    $status = if ($dev) { "$($dev.Status), problem code $($dev.Problem)" } else { 'no device found' }
    Write-Host "SudoVDA did NOT come up: $status. Check Device Manager > Display adapters." -ForegroundColor Red
    exit 1
}
