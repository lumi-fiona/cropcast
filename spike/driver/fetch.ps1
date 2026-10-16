# Downloads the Apollo v0.4.6 installer (NSIS) and extracts the signed SudoVDA package + nefconc.exe
# into spike\driver\pkg. No admin needed. Needs 7-Zip (it unpacks NSIS installers).
#   powershell -ExecutionPolicy Bypass -File spike\driver\fetch.ps1
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$url = 'https://github.com/ClassicOldSong/Apollo/releases/download/v0.4.6/Apollo-0.4.6.exe'
# Digest published by GitHub for the release asset.
$installerSha = '42B2AEFAACB3474511517A56B96EE9F0517F30AC38B5DD2FDA9FD5B478F5021A'
# Hashes of the files inside it, recorded 2026-10-16.
$expected = [ordered]@{
    'SudoVDA.dll' = '47EE263CB5DE9382C6630A2D7F3DAFEC4A49419F953BEEC869CA5DD0C460FF63'
    'SudoVDA.inf' = 'AD69AC682756F0CF339B081FAC7E6E8159FDF2CA01CA69DF8945C7246C286925'
    'sudovda.cat' = '2F9189DE5604BEC9D86F51640CC540639E394D9AD0F8E689129375E95F2D22F8'
    'sudovda.cer' = '6ACCDCD519F6179D967DB4EAA20ECF25A732BA30E87F4CFFEBC768B2C13C9007'
    'nefconc.exe' = '9DBA1F1A9E2B21843C4A0CA6C6FFA9E747250DD6E42C8C14325CA069AD43EA8F'
}

$sevenZip = @(
    (Get-Command 7z, 7za -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source),
    "$env:ProgramFiles\7-Zip\7z.exe",
    "${env:ProgramFiles(x86)}\7-Zip\7z.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $sevenZip) { throw '7-Zip not found. Install it (winget install 7zip.7zip) and run again.' }

$dl = Join-Path $here 'download'
$pkg = Join-Path $here 'pkg'
New-Item -ItemType Directory -Force $dl, $pkg | Out-Null
$installer = Join-Path $dl 'Apollo-0.4.6.exe'

if (-not (Test-Path $installer) -or (Get-FileHash $installer -Algorithm SHA256).Hash -ne $installerSha) {
    Write-Host "Downloading $url"
    $ProgressPreference = 'SilentlyContinue'  # the progress bar makes Invoke-WebRequest very slow on 5.1
    Invoke-WebRequest -Uri $url -OutFile $installer -UseBasicParsing
}
$got = (Get-FileHash $installer -Algorithm SHA256).Hash
if ($got -ne $installerSha) { throw "Installer SHA256 mismatch: got $got, expected $installerSha" }
Write-Host "Installer OK  $got  Apollo-0.4.6.exe"

# The package sits under drivers\sudovda\ in the NSIS payload. 'e' flattens the paths.
$names = $expected.Keys | ForEach-Object { "drivers\sudovda\$_" }
& $sevenZip e -y "-o$pkg" $installer @names | Out-Null
if ($LASTEXITCODE -ne 0) { throw "7-Zip failed with exit code $LASTEXITCODE" }

$bad = 0
foreach ($name in $expected.Keys) {
    $f = Join-Path $pkg $name
    if (-not (Test-Path $f)) { Write-Host "MISSING  $name" -ForegroundColor Red; $bad++; continue }
    $h = (Get-FileHash $f -Algorithm SHA256).Hash
    $ok = $h -eq $expected[$name]
    if (-not $ok) { $bad++ }
    Write-Host ("{0}  {1}  {2}" -f $(if ($ok) { 'OK      ' } else { 'MISMATCH' }), $h, $name)
}
$cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 (Join-Path $pkg 'sudovda.cer')
Write-Host "Certificate: $($cert.Subject), thumbprint $($cert.Thumbprint), valid $($cert.NotBefore) to $($cert.NotAfter)"
if ($bad) { throw "$bad file(s) missing or with an unexpected hash" }
Write-Host "Package ready in $pkg. Next (elevated): spike\driver\install.ps1"
