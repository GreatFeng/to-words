# Build the x64 Notepad-only TSF prototype with Visual Studio Build Tools.
$ErrorActionPreference = 'Stop'
$vcvars = 'C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Auxiliary\Build\vcvars64.bat'
if (-not (Test-Path -LiteralPath $vcvars)) { throw "未找到 VC 编译环境：$vcvars" }
$source = Split-Path -Parent $MyInvocation.MyCommand.Path
$out = Join-Path $source 'build'
New-Item -ItemType Directory -Path $out -Force | Out-Null
$command = 'call "' + $vcvars + '" >nul && cl /nologo /std:c++17 /EHsc /W4 /LD /Fe:"' + (Join-Path $out 'to_words_tsf_notepad_diag.dll') + '" "' + (Join-Path $source 'text_service.cpp') + '" /link /DEF:"' + (Join-Path $source 'text_service.def') + '" /OUT:"' + (Join-Path $out 'to_words_tsf_notepad_diag.dll') + '" && cl /nologo /std:c++17 /EHsc /W4 /Fe:"' + (Join-Path $out 'to_words_tsf_send.exe') + '" "' + (Join-Path $source 'send_to_notepad.cpp') + '" /link /OUT:"' + (Join-Path $out 'to_words_tsf_send.exe') + '" && cl /nologo /std:c++17 /EHsc /W4 /Fe:"' + (Join-Path $out 'tsf_register_probe.exe') + '" "' + (Join-Path $source 'register_probe.cpp') + '" /link /OUT:"' + (Join-Path $out 'tsf_register_probe.exe') + '" && cl /nologo /std:c++17 /EHsc /W4 /Fe:"' + (Join-Path $out 'tsf_profile_probe.exe') + '" "' + (Join-Path $source 'profile_probe.cpp') + '" /link /OUT:"' + (Join-Path $out 'tsf_profile_probe.exe') + '"'
Push-Location $out
try {
    & $env:ComSpec /d /s /c $command
    if ($LASTEXITCODE -ne 0) { throw "原型构建失败，退出码 $LASTEXITCODE" }
} finally {
    Pop-Location
}
