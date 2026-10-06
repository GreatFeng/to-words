# Delete only the unregistered Notepad test DLLs after the next Windows restart.
$ErrorActionPreference = 'Stop'
trap {
    [IO.File]::WriteAllText(
        (Join-Path $PSScriptRoot 'build\schedule_cleanup_error.txt'), ($_ | Out-String))
    exit 1
}

$principal = [Security.Principal.WindowsPrincipal]::new(
    [Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Administrator privileges are required.'
}

$clsid = '{45f1de8a-b258-4b62-a361-20478916e94c}'
$registrationKeys = @(
    "HKLM:\Software\Classes\CLSID\$clsid",
    "HKCU:\Software\Classes\CLSID\$clsid",
    "HKLM:\Software\Microsoft\CTF\TIP\$clsid",
    "HKCU:\Software\Microsoft\CTF\TIP\$clsid"
)
foreach ($key in $registrationKeys) {
    if (Test-Path -LiteralPath $key) {
        throw "Prototype registration still exists: $key"
    }
}

$installDir = 'C:\Program Files\to_words\tsf_notepad_prototype'
$expectedDir = [IO.Path]::GetFullPath($installDir).TrimEnd('\')
if ($expectedDir -cne $installDir) { throw 'Unexpected install directory.' }
if (-not (Test-Path -LiteralPath $expectedDir -PathType Container)) { exit 0 }

$files = @(Get-ChildItem -LiteralPath $expectedDir -Force)
foreach ($file in $files) {
    if ($file.PSIsContainer -or
        ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        $file.Name -notmatch '^to_words_tsf_notepad(?:_[0-9A-F]{12})?\.dll$' -or
        [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($file.FullName)) -ine $expectedDir) {
        throw "Unexpected item in test directory; refusing scheduled deletion: $($file.FullName)"
    }
}

Add-Type -Namespace ToWords -Name NativeCleanup -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("kernel32.dll", EntryPoint="MoveFileExW", CharSet=System.Runtime.InteropServices.CharSet.Unicode, SetLastError=true)]
public static extern bool MoveFileEx(string existingPath, System.IntPtr newPath, uint flags);
'@

$delayUntilReboot = [uint32]4
foreach ($file in $files) {
    if (-not [ToWords.NativeCleanup]::MoveFileEx($file.FullName, [IntPtr]::Zero, $delayUntilReboot)) {
        throw "Could not schedule file deletion: $($file.FullName); Win32=$([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
    }
    Write-Output "Scheduled: $($file.FullName)"
}
if (-not [ToWords.NativeCleanup]::MoveFileEx($expectedDir, [IntPtr]::Zero, $delayUntilReboot)) {
    throw "Could not schedule directory deletion: $expectedDir; Win32=$([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
}
Write-Output "Scheduled empty directory removal: $expectedDir"
