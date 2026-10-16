# Undoes install.ps1: removes the SudoVDA device node and driver package, then deletes the
# sudovda@su.mk certificate from LocalMachine Root and TrustedPublisher by thumbprint. MUST run elevated.
#   powershell -ExecutionPolicy Bypass -File spike\driver\uninstall.ps1
# Apollo uses the same driver and certificate; if Apollo is installed this refuses unless -Force.
param([switch]$Force)
$ErrorActionPreference = 'Stop'
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) {
    Write-Host 'uninstall.ps1 must run elevated: open PowerShell with "Run as administrator" and run it again.' -ForegroundColor Red
    exit 1
}
if ((Test-Path "$env:ProgramFiles\Apollo") -and -not $Force) {
    Write-Host 'Apollo is installed and shares this driver and certificate. Re-run with -Force to remove them anyway.' -ForegroundColor Yellow
    exit 1
}

$hwid = 'root\sudomaker\sudovda'
$classGuid = '4D36E968-E325-11CE-BFC1-08002BE10318'
$thumbprint = '3C918FC73525AD8B1521B6DB26B71F694277CC49'  # CN=sudovda@su.mk from Apollo 0.4.6
$nefconc = Join-Path $PSScriptRoot 'pkg\nefconc.exe'

Write-Host 'Removing the device node'
if (Test-Path $nefconc) {
    & $nefconc --remove-device-node --hardware-id $hwid --class-guid $classGuid
} else {
    Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.HardwareID -contains $hwid } |
        ForEach-Object { pnputil /remove-device $_.InstanceId }
}

Write-Host 'Removing the driver package from the driver store'
Get-WindowsDriver -Online | Where-Object { $_.OriginalFileName -like '*\sudovda.inf' } | ForEach-Object {
    Write-Host "  pnputil /delete-driver $($_.Driver)"
    pnputil /delete-driver $_.Driver /uninstall /force
}

foreach ($store in 'Root', 'TrustedPublisher') {
    $path = "Cert:\LocalMachine\$store\$thumbprint"
    if (Test-Path $path) {
        Remove-Item $path
        Write-Host "Deleted certificate $thumbprint from LocalMachine\$store"
    } else {
        Write-Host "Certificate $thumbprint not in LocalMachine\$store"
    }
}

$left = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.HardwareID -contains $hwid }
$certs = 'Root', 'TrustedPublisher' | Where-Object { Test-Path "Cert:\LocalMachine\$_\$thumbprint" }
if ($left -or $certs) { Write-Host "Something is left: device=$([bool]$left) certs=$($certs -join ',')" -ForegroundColor Red; exit 1 }
Write-Host 'SudoVDA and its certificate are gone.' -ForegroundColor Green
