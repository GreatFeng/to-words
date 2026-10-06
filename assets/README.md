# Assets

本目录集中存放不会直接写进 Rust 源码的静态资源：

- `fonts/`：界面或词库编辑器使用的字体。
- `icons/`：程序、托盘和按钮图标。
- `images/`：说明图片或其他位图资源。
- `ocr/`：可选的便携式 Tesseract 引擎及语言包，详见该目录的 README。
- `voice/`：可选的本地语音识别引擎和多语言模型，详见该目录的 README。

新增资源后，应使用相对于项目根目录的路径加载；发布版本时需要把实际使用的资源一并复制到可执行文件目录。

## 当前图标资源

- `icons/to_words_tray_source.png`：托盘图标的高分辨率设计源文件。
- `icons/to_words_tray_32.png`：用于检查实际托盘尺寸效果的 32×32 PNG。
- `icons/to_words_tray_32.rgba`：编译进程序并交给 `tray_icon::Icon::from_rgba` 的 RGBA 像素数据。

托盘图标已经通过 `include_bytes!` 嵌入可执行文件，发布时不需要额外携带图标文件。
