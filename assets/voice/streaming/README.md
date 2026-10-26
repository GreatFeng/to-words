# 流式字幕模型

将 sherpa-onnx 官方的 `sherpa-onnx-streaming-zipformer-small-bilingual-zh-en-2023-02-16` 模型中这四个文件放到 `zipformer-small-zh-en/`：

```text
assets/voice/streaming/zipformer-small-zh-en/
  encoder-epoch-99-avg-1.int8.onnx
  decoder-epoch-99-avg-1.onnx
  joiner-epoch-99-avg-1.int8.onnx
  tokens.txt
```

下载地址：[官方发布包](https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-streaming-zipformer-small-bilingual-zh-en-2023-02-16.tar.bz2)。只需上述四个文件，合计约 60 MB。流式模型负责中英文临时字幕；句末仍由 SenseVoice 修正。韩语、日语目前没有内置的流式预览模型，仍由 SenseVoice 出最终字幕。
