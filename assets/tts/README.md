# 本地发音模型

`to_words` 会扫描程序目录下的 `assets/tts/`，把完整的 Piper/VITS 和 Supertonic 模型加入「设置 → 语音识别 → 查询结果发音」列表。
每个模型可放在自己的子目录中；启动后打开设置即可选择其对应语言的声音。自动麦克风发音会使用同一个发音语音配置，将合成声音送到设置中指定的虚拟音频播放设备。

Huayan（简体中文）的目录结构：

```text
assets/tts/huayan/vits-piper-zh_CN-huayan-medium/
├─ zh_CN-huayan-medium.onnx
├─ zh_CN-huayan-medium.onnx.json
├─ tokens.txt
└─ espeak-ng-data/
```

此处需要 [sherpa-onnx 兼容版 Huayan 模型包](https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-zh_CN-huayan-medium.tar.bz2)。原始 Piper 仓库中的 `.onnx` 缺少 sherpa-onnx 需要的元数据，仅有模型和 JSON 文件无法用于当前播放接口。下载并解压完整包后重启程序。

模型约 81 MB，未提交到源码仓库。打包时按需将此目录随程序一同分发。模型卡注明训练数据集的许可状态为 Unknown；对外分发前应确认相应授权。

## 已添加的声音

| 语言 | 模型 | 安装目录 | 官方下载 |
| --- | --- | --- | --- |
| 中文 | Chaowen（Piper medium） | `vits-piper-zh_CN-chaowen-medium/` | [Sherpa-ONNX 模型包](https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-zh_CN-chaowen-medium.tar.bz2) |
| 英文 | Lessac（Piper high） | `vits-piper-en_US-lessac-high/` | [Sherpa-ONNX 模型包](https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-en_US-lessac-high.tar.bz2) |
| 韩文、日文、英文 | Supertonic 3（每种语言 10 种声线） | `sherpa-onnx-supertonic-3-tts-int8-2026-05-11/` | [Sherpa-ONNX 模型包](https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/sherpa-onnx-supertonic-3-tts-int8-2026-05-11.tar.bz2) |

模型文件在本机，不提交到 Git。发布新版本时，请把选用的模型目录随 `to_words.exe` 一起复制到 `assets/tts/`。Supertonic 使用 CPU 合成，初次选择时加载模型可能稍慢。对外分发模型前，请分别核对原始模型及训练数据的许可条款；尤其不要仅依据转换包里的 `LICENSE` 推断原模型授权。
