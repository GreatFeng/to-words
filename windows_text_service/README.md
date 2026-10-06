# 记事本 TSF 文本服务原型（仅开发测试）

本目录是 `to_words` 的可选 Windows 文本服务原型。它只在 `notepad.exe` 进程中建立接收窗口，通过 TSF 编辑会话把文字写到记事本当前选区；不使用剪贴板，也不模拟 Ctrl+V。**目前仅在记事本验证，不应在 Aion2 中启用或测试。**普通版程序不需要安装这个组件。

## 当前验证结果

- 此前测试时注册了 `to_words (Notepad test)`，从未设为默认输入法；测试安装现已卸载。
- 此前使用当前用户的 30 天开发测试证书为受保护目录中的 DLL 签名，记事本成功加载了 DLL；测试证书现已移除。
- 直接发送 `TSF_OK` 的测试已在记事本正文中可见。最初的 `ITfInsertAtSelection` 调用返回 `0x80070057`；改用 `ITfContext::GetSelection` + `ITfRange::SetText` 后，编辑步骤均返回成功。
- 带 `tsf-notepad-prototype` feature 的完整语音输入链路已由用户在记事本中验证，词库结果可直接落字。记事本成功不代表游戏一定支持。

## 构建与测试

需要 Visual Studio C++ Build Tools 和 Windows SDK。运行 `.\windows_text_service\build.ps1` 构建诊断 DLL、发送工具和状态探针。只有 `cargo run --features tsf-notepad-prototype` 会让 Rust 程序在前台为记事本时使用 TSF 路径；普通版不走此原型。

若以后需要重新安装：先运行 `.\windows_text_service\build.ps1`，确认 Aion2 已关闭，再运行 `.\windows_text_service\registration.ps1` 完成基础注册，最后用管理员 PowerShell 运行 `.\windows_text_service\protected_test_install.ps1`。测试 DLL 安装在 `C:\Program Files\to_words\tsf_notepad_prototype`。重新编译 DLL 后，可用该脚本的 `-Upgrade` 参数签名并安装新版本，不必覆盖仍被 Windows 进程加载的旧 DLL。日志只包含进程名、阶段和 HRESULT，不包含语音、原文或译文，写在 `%TEMP%\to_words_tsf_notepad.log`。

测试时先确认 Aion2 已关闭，再打开空白记事本，在输入法列表中选 `to_words (Notepad test)`。`build\to_words_tsf_send.exe TSF_OK` 可测试直接写入；做语音测试时运行上述带 feature 的 `cargo run`，保持记事本编辑区为前台。

结束测试时先切回平常输入法，保存并关闭记事本，然后用管理员 PowerShell 运行 `.\windows_text_service\protected_test_install.ps1 -Uninstall`。脚本只注销本原型的 COM/TSF 项、移除当前用户测试证书和自己的安装 DLL。若资源管理器等进程仍加载旧 DLL，可用管理员 PowerShell 运行 `.\windows_text_service\schedule_locked_cleanup.ps1`，只把本原型残留 DLL 和目录登记为下次重启删除；或者重启后运行卸载脚本的 `-CleanFiles` 参数。不要删除其他输入法注册项。

初次未使用受保护部署时，`registration.ps1` 可注册或注销工作区 DLL；受保护版本已安装时，该脚本会拒绝运行，以免把 COM 路径退回可写的工作区。
