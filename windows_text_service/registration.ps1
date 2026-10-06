param([switch]$Unregister)

# Register/unregister only this prototype's GUIDs. Requires a visible UAC consent prompt.
$ErrorActionPreference = 'Stop'
if (Get-Process -Name Aion2 -ErrorAction SilentlyContinue) {
    throw '请先关闭 Aion2；本原型只允许在记事本中测试。'
}
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$dll = Join-Path $root 'build\to_words_tsf_notepad_diag.dll'
$helper = Join-Path $root 'build\tsf_register_probe.exe'
if (Test-Path -LiteralPath 'HKLM:\Software\Classes\CLSID\{45f1de8a-b258-4b62-a361-20478916e94c}') {
    throw 'Protected Notepad prototype is installed. Use protected_test_install.ps1 for upgrades or removal.'
}
if (-not (Test-Path -LiteralPath $dll) -or -not (Test-Path -LiteralPath $helper)) {
    throw '原型尚未构建，请先运行 windows_text_service\build.ps1。'
}
$action = if ($Unregister) { 'unregister' } else { 'register' }
$process = Start-Process -FilePath $helper -ArgumentList @($action, "`"$dll`"") -Verb RunAs -PassThru -Wait -WindowStyle Hidden
if ($process.ExitCode -ne 0) {
    throw "TSF $action 失败，退出码 $($process.ExitCode)。请勿在失败后启用该输入法。"
}
$class = 'HKCU:\Software\Classes\CLSID\{45f1de8a-b258-4b62-a361-20478916e94c}\InprocServer32'
$machine = 'HKLM:\Software\Microsoft\CTF\TIP\{45f1de8a-b258-4b62-a361-20478916e94c}'
Write-Output "用户 COM 注册：$(Test-Path -LiteralPath $class)"
Write-Output "系统 TSF 注册：$(Test-Path -LiteralPath $machine)"
if ($Unregister) {
    if ((Test-Path -LiteralPath $class) -or (Test-Path -LiteralPath $machine)) {
        throw '注销后仍有原型注册项，请检查管理员权限和 Windows 输入法状态。'
    }
} elseif (-not (Test-Path -LiteralPath $class) -or -not (Test-Path -LiteralPath $machine)) {
    throw '注册未完整写入，请勿继续测试。'
}
