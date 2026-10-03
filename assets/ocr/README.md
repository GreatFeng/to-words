# 可选的便携式 OCR 资源

可把 `tesseract.exe`、它运行所需的 DLL 与 `tessdata/` 放在此目录。程序会优先使用此处的引擎；未提供时会查找系统安装的 Tesseract。

不要只复制 `tesseract.exe`：缺少 DLL 或 `tessdata/*.traineddata` 时无法识别。此目录默认不随源码提供第三方二进制文件。
