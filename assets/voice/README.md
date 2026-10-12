# 本地语音识别资源

默认语音引擎是 SenseVoice。请把 `funasr-llamacpp-windows-x64.zip` **解压后的内容**放在项目的 `assets/voice/funasr/`；发布版则放在 `to_words.exe` 旁的同名目录。将 `sensevoice-small-q8.gguf` 和 `fsmn-vad.gguf` 也放到这里。最终至少应有：

```text
assets/voice/funasr/
├── llama-funasr-sensevoice.exe
├── sensevoice-small-q8.gguf
├── fsmn-vad.gguf
└── 该压缩包附带的其他 DLL（如有）
```

不要只放 ZIP，也不要多套一层解压目录；程序会按上面的固定路径查找文件。所有语言组合默认使用 SenseVoice，不会自动切换或回退到 Whisper。SenseVoice 本身会自动识别语种；当前独立程序不支持固定识别语言。同语言模式会过滤明显不符的识别文字，但不能保证只识别指定语言。只有在设置中手动选择 Whisper 时，才需要在 `assets/voice/whisper/` 放置 Whisper 程序、多语言模型及其依赖 DLL。该 Windows x64 SenseVoice 包使用 CPU；识别速度仍取决于机器性能。项目不提交第三方引擎与模型，发布时须自行随程序打包。

Whisper.cpp 仍可在设置中切换使用。它的程序、模型和依赖 DLL 统一放在 `assets/voice/whisper/`（发布版则放在 `to_words.exe` 旁的同名目录）：

- `whisper-cli.exe`：来自 [whisper.cpp 的正式发布页](https://github.com/ggml-org/whisper.cpp/releases) 的 Windows 命令行识别程序。
- `ggml-base.bin`：从 [whisper.cpp 多语言模型文件页](https://huggingface.co/ggerganov/whisper.cpp/blob/main/ggml-base.bin) 下载。也支持 `ggml-small.bin`、`ggml-medium.bin` 或 `ggml-tiny.bin`；不要选文件名包含 `.en` 的英文专用模型，否则中文、韩文、日文不能正常识别。

想改善 Whisper 的口语识别准确度，建议先下载 [ggml-small.bin](https://huggingface.co/ggerganov/whisper.cpp/blob/main/ggml-small.bin) 放到 `assets/voice/whisper/`；程序会自动优先使用它，无需删除现有 `ggml-base.bin`。如果机器性能充足，也可以放入 `ggml-medium.bin`，它的优先级更高，但识别会明显变慢。设置面板会显示当前选用的模型。经常使用固定语言时，在设置中的“语音识别 → 识别语言”里指定该语言；默认仍为自动识别。

如果 `whisper-cli.exe` 依赖同目录的 DLL，须一并复制到这里。可从官方项目构建 Windows 版本，或使用其正式发布的适用版本。源码仓库不包含这些较大的第三方二进制文件；发布便携版时要把它们随程序一起打包。设置页会提示缺失的文件。

测试录音单独放在 `assets/voice/test_samples/`，例如 `test-voice2.wav`；它不参与正常识别，也不随源码提交。整理后的目录结构如下：

```text
assets/voice/
├── funasr/          # SenseVoice 程序、模型和 VAD
├── whisper/         # Whisper 程序、GGML 模型和依赖 DLL
├── speaker/         # 可选：本人声纹识别模型
└── test_samples/    # 本机测试录音
```

本人声音过滤为可选功能，默认关闭。使用前请从 [sherpa-onnx 官方说话人模型发布页](https://github.com/k2-fsa/sherpa-onnx/releases/tag/speaker-recongition-models) 下载 `3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx`，放在 `assets/voice/speaker/`（发布版同样放在 `to_words.exe` 旁的 `assets/voice/speaker/`）。打开设置 → 语音识别，先确定“过滤电脑播放声”是否开启，再用日常麦克风在实际使用环境录制三段各至少 1.5 秒的不同句子，每段之间停顿约一秒。完成后勾选“仅识别已注册的本人声音”并保存。切换播放声过滤设置或更换麦克风后请重新注册，避免本人语音被拒。只保存模型提取的向量到 `to_words.db`，不保存注册录音；本地数据库文件仍应妥善保管。声纹过滤不是可靠的身份认证，录音回放、多人同时说话或环境变化都可能误判。

“过滤电脑播放声”也默认关闭。启用后优先尝试 Windows 默认**通信麦克风**的声学回声消除（AEC）；设备不支持时，改用默认播放设备的 WASAPI 回采作为参考，通过软件回声消除处理麦克风声音。无需麦克风内置 AEC，但软件路径会增加运算量，效果取决于设备延迟与音量，仍需在实际设备上验证。使用耳机、降低扬声器音量通常也有帮助。此功能只减轻电脑播放声漏入麦克风，不能消除手机外放、旁人说话或所有回声。

本人声纹过滤支持不足一秒的短词，并尝试在有明显停顿的轮流说话中逐段保留本人声音。多人同时重叠讲话没有做真正的声源分离，仍可能误拒或误收；声纹过滤不能用于身份认证。

Windows 还需要允许桌面应用使用麦克风。默认按一次语音快捷键开始录音，说完后静音达到设置时间（默认 0.6 秒，可调 0.2–3.0 秒）会自动提交并停止，也可再按一次提前停止。勾选“是否保持语音输入”后，每句结束会自动识别并继续监听，直到再次按快捷键。勾选“按住语音快捷键说话”后，按住期间持续录音，松开主按键后停止并识别，不按停顿自动提交。非保持模式单次录音最多约 90 秒。录音在本机临时目录中短暂写成 WAV 供模型识别，识别结束后立即删除；只有词库未命中且 DeepSeek 已启用时，识别出的文字才会发送到 DeepSeek。

第三方语音 CLI 可能无法读取中文等非英文路径。程序优先尝试 Windows 短路径；不可用时会在系统临时目录的 `to_words_voice_runtime` 中缓存当前引擎所需的程序、模型和 DLL，并使用英文路径存放临时 WAV。相同卷优先创建硬链接，跨卷时首次需要复制模型，因此可能多占数百 MB，后续识别复用缓存。若系统临时目录自身也没有可用英文路径，设置页会提示修改 Windows 的 TEMP/TMP 为可写的英文路径。

在 Windows 上，持续监听模式会以低于正常的进程优先级运行识别程序，并把 Whisper 的识别线程限制为最多 4 个，减少与游戏争抢 CPU；代价是识别结果可能稍晚出现。单次识别和按住说话不受这项限制。如果游戏仍卡顿，可先关闭持续监听做对照，再检查是否启用了额外消耗 CPU 的播放声过滤、本人声音过滤，或使用了较大的 Whisper 模型。
