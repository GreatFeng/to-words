# 本地语音识别资源

语音识别需要两个文件，放在本目录（发布版则放在 `to_words.exe` 旁的 `assets/voice/`）中：

- `whisper-cli.exe`：来自 [whisper.cpp 的正式发布页](https://github.com/ggml-org/whisper.cpp/releases) 的 Windows 命令行识别程序。
- `ggml-base.bin`：从 [whisper.cpp 多语言模型文件页](https://huggingface.co/ggerganov/whisper.cpp/blob/main/ggml-base.bin) 下载。也支持 `ggml-small.bin`、`ggml-medium.bin` 或 `ggml-tiny.bin`；不要选文件名包含 `.en` 的英文专用模型，否则中文、韩文、日文不能正常识别。

想改善口语识别准确度，建议先下载 [ggml-small.bin](https://huggingface.co/ggerganov/whisper.cpp/blob/main/ggml-small.bin) 放到本目录；程序会自动优先使用它，无需删除现有 `ggml-base.bin`。如果机器性能充足，也可以放入 `ggml-medium.bin`，它的优先级更高，但识别会明显变慢。设置面板会显示当前选用的模型。经常使用固定语言时，在设置中的“语音识别 → 识别语言”里指定该语言；默认仍为自动识别。

如果 `whisper-cli.exe` 依赖同目录的 DLL，须一并复制到这里。可从官方项目构建 Windows 版本，或使用其正式发布的适用版本。源码仓库不包含这些较大的第三方二进制文件；发布便携版时要把它们随程序一起打包。设置页会提示缺失的文件。

Windows 还需要允许桌面应用使用麦克风。默认按一次语音快捷键开始录音，说完后约一秒静音会自动提交并停止，也可再按一次提前停止。勾选“是否保持语音输入”后，每句结束会自动识别并继续监听，直到再次按快捷键。非保持模式单次录音最多约 90 秒。录音在本机临时目录中短暂写成 WAV 供模型识别，识别结束后立即删除；只有词库未命中且 DeepSeek 已启用时，识别出的文字才会发送到 DeepSeek。

Windows 本地 AI 识别（实验版）是另一个可选引擎，不需要本目录的 Whisper 模型，但需要 `to_words_windows_speech.exe` 及其 .NET 发布依赖、已注册的 MSIX 包身份和 Windows 语音模型。具体构建和安装见 [开发者文档](../../docs/DEVELOPER.md)。它并不直接调用 Win+H 语音输入。
