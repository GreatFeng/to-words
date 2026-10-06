# Builds a writable external folder and an unsigned sparse MSIX identity package.
# Signing, certificate trust, and Add-AppxPackage registration are intentionally manual.
param([switch]$SkipRustBuild)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$env:DOTNET_CLI_HOME = Join-Path $repo 'target\dotnet-home'
$env:NUGET_PACKAGES = Join-Path $repo 'target\nuget-packages'
$env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = '1'
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$sdkBins = Get-ChildItem -LiteralPath 'C:\Program Files (x86)\Windows Kits\10\bin' -Directory |
    Where-Object { $_.Name -match '^10\.0\.\d+\.0$' } |
    Sort-Object { [version]$_.Name } -Descending
$sdkBin = $sdkBins | ForEach-Object { Join-Path $_.FullName 'x64' } |
    Where-Object { (Test-Path (Join-Path $_ 'mt.exe')) -and (Test-Path (Join-Path $_ 'makeappx.exe')) } |
    Select-Object -First 1
if (-not $sdkBin) { throw 'Windows SDK mt.exe / makeappx.exe not found.' }
if (-not (Get-Command dotnet -ErrorAction SilentlyContinue) -or -not (dotnet --list-sdks)) {
    throw 'Install .NET SDK 8 or newer; .NET Runtime alone is insufficient.'
}

$stage = Join-Path $repo ('target\to_words_msix_stage_' + (Get-Date -Format 'yyyyMMdd_HHmmss'))
New-Item -ItemType Directory -Path $stage -ErrorAction Stop | Out-Null
$voice = Join-Path $stage 'assets\voice'
$icons = Join-Path $stage 'assets\icons'
$identity = Join-Path $stage 'identity'
New-Item -ItemType Directory -Path (Join-Path $stage 'assets') | Out-Null
New-Item -ItemType Directory -Path $voice, $icons, $identity | Out-Null

Push-Location $repo
try {
    if (-not $SkipRustBuild) {
        cargo build --release --offline -j 1 --features windows-speech
        if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed.' }
    }
    if (-not (Test-Path -LiteralPath 'target\release\to_words.exe')) {
        throw 'target\release\to_words.exe not found; run cargo build --release first.'
    }
    dotnet restore 'windows_speech_bridge\WindowsSpeechBridge.csproj' `
        --configfile 'windows_speech_bridge\NuGet.Config' -r win-x64 -p:SelfContained=true
    if ($LASTEXITCODE -ne 0) { throw 'Windows speech bridge restore failed.' }
    dotnet publish 'windows_speech_bridge\WindowsSpeechBridge.csproj' -c Release -r win-x64 `
        --self-contained true -p:PublishSingleFile=false -o $voice --no-restore
    if ($LASTEXITCODE -ne 0) { throw 'Windows speech bridge publish failed.' }

    Copy-Item -LiteralPath 'target\release\to_words.exe' -Destination $stage
    Copy-Item -LiteralPath 'ui_config.json' -Destination $stage
    Copy-Item -LiteralPath 'assets\icons\to_words_tray_32.png' -Destination $icons
    Get-ChildItem -LiteralPath 'assets\voice' -File |
        Where-Object { $_.Extension -in '.exe', '.dll', '.bin' -and $_.Name -ne 'to_words_windows_speech.exe' } |
        Copy-Item -Destination $voice
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'AppxManifest.xml') -Destination $identity

    $toWordsExe = Join-Path $stage 'to_words.exe'
    $bridgeExe = Join-Path $voice 'to_words_windows_speech.exe'
    & (Join-Path $sdkBin 'mt.exe') -manifest (Join-Path $PSScriptRoot 'to_words.manifest') `
        "-outputresource:$toWordsExe;#1"
    if ($LASTEXITCODE -ne 0) { throw 'Embedding MSIX identity into to_words.exe failed.' }
    & (Join-Path $sdkBin 'mt.exe') -manifest (Join-Path $PSScriptRoot 'bridge.manifest') `
        "-outputresource:$bridgeExe;#1"
    if ($LASTEXITCODE -ne 0) { throw 'Embedding MSIX identity into the speech bridge failed.' }

    $package = Join-Path $stage 'ToWordsSpeech.identity-unsigned.msix'
    & (Join-Path $sdkBin 'makeappx.exe') pack /d $identity /nv /p $package
    if ($LASTEXITCODE -ne 0) { throw 'MSIX identity package build failed.' }
    Write-Output "Stage directory: $stage"
    Write-Output "Unsigned identity package: $package"
    Write-Output 'Next: sign the MSIX, trust its signing certificate, then register it with Add-AppxPackage -ExternalLocation.'
} finally {
    Pop-Location
}
