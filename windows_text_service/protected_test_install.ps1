param([switch]$Uninstall, [switch]$Upgrade, [switch]$CleanFiles)

# Local Notepad-only TSF prototype. Run elevated; never use this for game testing.
$ErrorActionPreference = 'Stop'
trap {
    $message = $_ | Out-String
    [IO.File]::WriteAllText((Join-Path $PSScriptRoot 'build\protected_install_error.txt'), $message)
    exit 1
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Administrator privileges are required.'
}
if (Get-Process -Name Aion2 -ErrorAction SilentlyContinue) {
    throw 'Aion2 is running. This prototype is for Notepad only.'
}

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$source = Join-Path $root 'build\to_words_tsf_notepad_diag.dll'
$installDir = 'C:\Program Files\to_words\tsf_notepad_prototype'
$installedDll = Join-Path $installDir 'to_words_tsf_notepad.dll'
$clsid = '{45f1de8a-b258-4b62-a361-20478916e94c}'
$machineKey = "HKLM:\Software\Classes\CLSID\$clsid"
$userKey = "HKCU:\Software\Classes\CLSID\$clsid"
$subject = 'CN=to_words Notepad TSF Prototype (Local Test)'
$probe = Join-Path $root 'build\tsf_register_probe.exe'
if (([int]$Uninstall.IsPresent + [int]$Upgrade.IsPresent + [int]$CleanFiles.IsPresent) -gt 1) {
    throw 'Choose only one of Uninstall, Upgrade, or CleanFiles.'
}

function Remove-PrototypeFiles {
    if (-not (Test-Path -LiteralPath $installDir)) { return }
    $expectedParent = [IO.Path]::GetFullPath($installDir).TrimEnd('\')
    $leftovers = @()
    foreach ($file in Get-ChildItem -LiteralPath $installDir -File) {
        if (($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            $file.Name -notmatch '^to_words_tsf_notepad(?:_[0-9A-F]{12})?\.dll$' -or
            [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($file.FullName)) -ine $expectedParent) {
            throw "Unexpected file in test directory; refusing further removal: $($file.FullName)"
        }
        try { Remove-Item -LiteralPath $file.FullName -ErrorAction Stop }
        catch { $leftovers += $file.FullName }
    }
    if ($leftovers.Count -eq 0) {
        Remove-Item -LiteralPath $installDir
    } else {
        Write-Output "Files still loaded; retry CleanFiles after restarting Windows: $($leftovers -join ', ')"
    }
}

if ($CleanFiles) {
    if ((Test-Path -LiteralPath $machineKey) -or (Test-Path -LiteralPath $userKey) -or
        (Test-Path -LiteralPath "HKLM:\Software\Microsoft\CTF\TIP\$clsid")) {
        throw 'Prototype registration still exists; refusing file cleanup.'
    }
    Remove-PrototypeFiles
    exit 0
}

if ($Uninstall) {
    $activeDll = (Get-Item -LiteralPath (Join-Path $machineKey 'InprocServer32') -ErrorAction Stop).GetValue('')
    $expectedParent = [IO.Path]::GetFullPath($installDir).TrimEnd('\')
    if ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($activeDll)) -ine $expectedParent -or
        [IO.Path]::GetFileName($activeDll) -notmatch '^to_words_tsf_notepad(?:_[0-9A-F]{12})?\.dll$') {
        throw "Unexpected registered DLL path; refusing removal: $activeDll"
    }
    if (-not (Test-Path -LiteralPath $activeDll)) { throw "Test DLL not found: $activeDll" }
    $thumbprint = (Get-ItemProperty -LiteralPath $machineKey -Name TestCertThumbprint -ErrorAction Stop).TestCertThumbprint
    if ($thumbprint -notmatch '^[0-9A-F]{40}$') { throw 'Invalid test certificate thumbprint.' }
    if (-not (Test-Path -LiteralPath $probe)) { throw "Unregister helper missing: $probe" }
    & $probe unregister $activeDll
    if ($LASTEXITCODE -ne 0) { throw 'TSF unregister failed. Cleanup stopped.' }
    if (Test-Path -LiteralPath $machineKey) {
        Remove-Item -LiteralPath $machineKey -Recurse
    }
    foreach ($storeName in @('TrustedPublisher', 'Root', 'My')) {
        $store = [Security.Cryptography.X509Certificates.X509Store]::new(
            $storeName, [Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser)
        $store.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
        try {
            $cert = $store.Certificates.Find(
                [Security.Cryptography.X509Certificates.X509FindType]::FindByThumbprint,
                $thumbprint, $false) | Where-Object Subject -eq $subject
            foreach ($item in $cert) { $store.Remove($item) }
        } finally { $store.Close() }
    }
    Remove-PrototypeFiles
    Write-Output 'Notepad prototype COM/TSF registration and current-user test certificate removed.'
    exit 0
}

if ($Upgrade) {
    if (-not (Test-Path -LiteralPath $source) -or
        -not (Test-Path -LiteralPath $installedDll)) {
        throw 'Source or installed test DLL is missing.'
    }
    $thumbprint = (Get-ItemProperty -LiteralPath $machineKey -Name TestCertThumbprint -ErrorAction Stop).TestCertThumbprint
    if ($thumbprint -notmatch '^[0-9A-F]{40}$') { throw 'Invalid test certificate thumbprint.' }
    $cert = Get-ChildItem 'Cert:\CurrentUser\My' |
        Where-Object { $_.Thumbprint -eq $thumbprint -and $_.Subject -eq $subject -and $_.HasPrivateKey } |
        Select-Object -First 1
    if (-not $cert) { throw 'Test signing certificate with private key is missing.' }
    $signTool = 'C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\signtool.exe'
    & $signTool sign /fd SHA256 /sha1 $thumbprint /s My $source
    if ($LASTEXITCODE -ne 0) { throw 'Upgrade DLL signing failed.' }
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        $signature = Get-AuthenticodeSignature -LiteralPath $source
        if ($signature.Status -eq 'Valid' -and
            $signature.SignerCertificate.Thumbprint -eq $thumbprint) { break }
        Start-Sleep -Milliseconds 500
    }
    if ($signature.Status -ne 'Valid') { throw "Upgrade signature invalid: $($signature.Status)" }
    $sourceHash = (Get-FileHash -LiteralPath $source).Hash
    $versionedDll = Join-Path $installDir ("to_words_tsf_notepad_$($sourceHash.Substring(0, 12)).dll")
    if (Test-Path -LiteralPath $versionedDll) {
        if ((Get-FileHash -LiteralPath $versionedDll).Hash -ne $sourceHash) {
            throw 'Versioned target exists with a different hash.'
        }
    } else {
        Copy-Item -LiteralPath $source -Destination $versionedDll -ErrorAction Stop
    }
    if ($sourceHash -ne (Get-FileHash -LiteralPath $versionedDll).Hash) {
        throw 'Installed DLL hash does not match signed source.'
    }
    Set-Item -LiteralPath (Join-Path $machineKey 'InprocServer32') -Value $versionedDll
    Set-Item -LiteralPath (Join-Path $userKey 'InprocServer32') -Value $versionedDll
    Write-Output "Upgraded signed Notepad-only test DLL: $versionedDll"
    exit 0
}

if (-not (Test-Path -LiteralPath $source)) { throw "Diagnostic DLL missing: $source" }
if (Test-Path -LiteralPath $installDir) { throw "Target directory exists; refusing overwrite: $installDir" }
if (Test-Path -LiteralPath $machineKey) { throw "Machine COM key exists; refusing overwrite: $machineKey" }

$cert = Get-ChildItem 'Cert:\CurrentUser\My' |
    Where-Object { $_.Subject -eq $subject -and $_.HasPrivateKey -and $_.NotAfter -gt (Get-Date) } |
    Sort-Object NotAfter -Descending | Select-Object -First 1
if (-not $cert) {
    $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $subject `
        -FriendlyName 'to_words Notepad prototype - local test only' `
        -CertStoreLocation 'Cert:\CurrentUser\My' -KeyExportPolicy NonExportable `
        -HashAlgorithm SHA256 -NotAfter (Get-Date).AddDays(30)
}
foreach ($storeName in @('Root', 'TrustedPublisher')) {
    $store = [Security.Cryptography.X509Certificates.X509Store]::new(
        $storeName, [Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser)
    $store.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
    try {
        $existing = $store.Certificates.Find(
            [Security.Cryptography.X509Certificates.X509FindType]::FindByThumbprint,
            $cert.Thumbprint, $false)
        if ($existing.Count -eq 0) { $store.Add($cert) }
    } finally { $store.Close() }
}

$signTool = 'C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\signtool.exe'
if (-not (Test-Path -LiteralPath $signTool)) { throw "SignTool missing: $signTool" }
$signature = Get-AuthenticodeSignature -LiteralPath $source
if ($signature.SignerCertificate.Thumbprint -ne $cert.Thumbprint -or $signature.Status -ne 'Valid') {
    & $signTool sign /fd SHA256 /sha1 $cert.Thumbprint /s My $source
    if ($LASTEXITCODE -ne 0) { throw 'Development signing failed.' }
}
for ($attempt = 0; $attempt -lt 20; $attempt++) {
    $signature = Get-AuthenticodeSignature -LiteralPath $source
    if ($signature.Status -eq 'Valid' -and
        $signature.SignerCertificate.Thumbprint -eq $cert.Thumbprint) { break }
    Start-Sleep -Milliseconds 500
}
if ($signature.Status -ne 'Valid') {
    throw "Signature verification failed: $($signature.Status): $($signature.StatusMessage)"
}

New-Item -ItemType Directory -Path $installDir -Force | Out-Null
Copy-Item -LiteralPath $source -Destination $installedDll -ErrorAction Stop
$acl = Get-Acl -LiteralPath $installedDll
$writable = $acl.Access | Where-Object {
    $_.AccessControlType -eq 'Allow' -and
    $_.IdentityReference.Value -match '(^|\\)(Users|Authenticated Users|Everyone)$' -and
    ($_.FileSystemRights -band [Security.AccessControl.FileSystemRights]::Write)
}
if ($writable) { throw 'Install DLL is writable by ordinary users; registration stopped.' }

New-Item -Path $machineKey -Force | Out-Null
Set-Item -LiteralPath $machineKey -Value 'to_words Notepad-only TSF prototype'
Set-ItemProperty -LiteralPath $machineKey -Name TestCertThumbprint -Value $cert.Thumbprint
$machineServer = Join-Path $machineKey 'InprocServer32'
New-Item -Path $machineServer -Force | Out-Null
Set-Item -LiteralPath $machineServer -Value $installedDll
Set-ItemProperty -LiteralPath $machineServer -Name ThreadingModel -Value 'Apartment'

# HKCU registration takes priority over HKLM for this interactive user.
$userServer = Join-Path $userKey 'InprocServer32'
if (-not (Test-Path -LiteralPath $userServer)) {
    throw 'Current-user prototype COM key is missing; deployment stopped.'
}
Set-Item -LiteralPath $userServer -Value $installedDll
Write-Output "Installed signed Notepad-only test DLL: $installedDll"
Write-Output "Certificate thumbprint: $($cert.Thumbprint)"
