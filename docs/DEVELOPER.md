# to_words 开发者文档

本文保留原 README 中的功能说明、词库格式和构建发布步骤；普通用户请阅读 [使用说明](../README.md)。模块职责与依赖方向见 [项目结构](ARCHITECTURE.md)。

`to_words` 是一个 Windows 桌面快捷词查询工具。通过全局快捷键打开查询框，输入中文、韩文或其他文字，即可从当前选择的用户词库中搜索词条、组合结果并复制到剪贴板。

## 运行

安装 Rust 后，在项目目录执行：

```powershell
cargo run
```

程序启动后在后台运行，不会自动弹出查询框。系统托盘出现图标即表示程序已经启动。

默认快捷键：

- `Ctrl + Alt + Enter`：打开查询框
- `Ctrl + Alt + S`：打开设置面板
- `Ctrl + Alt + O`：框选屏幕文字；松开后预览本地 OCR 与译文，点击 `√` 复制识别原文并关闭
- `Ctrl + Alt + V`：开始或结束本地语音识别
- `Ctrl + Backspace`：查询框为空时清空全部组合内容

快捷键均可在设置页面重新录制。

### 本地语音识别

语音模块见 `src/platform/voice.rs`，使用 Windows `waveIn` 以 16 kHz 单声道录音，并调用随程序发布的 `whisper-cli.exe` 和多语言 GGML 模型。录音/停顿检测与语音识别使用独立线程，上一句识别时仍能检测下一句，识别任务按语句顺序处理；每 100 毫秒分析一帧音量，开头约 0.4 秒估计背景噪音，说话中根据语音峰值动态判断停顿，检测到约 0.9 秒停顿后只识别完整句子一次，不再每四秒重复启动模型。设置项 `voice_keep_input` 默认 `false`；开启后，每句自动提交并继续监听，直到再次按快捷键结束。语音活动检测仍是音量阈值启发式算法，持续人声或音乐等复杂噪声可能需要手动停止。悬浮文字由 `src/components/voice_overlay.rs` 的独立透明、置顶、鼠标穿透视口绘制，始终位于屏幕底部；最终结果显示约 1 秒后消失，即使保持语音输入也不会一直占据屏幕，下一句开始时清空旧文字。`src/platform/voice_output.rs` 在每条结果产生时捕获当前外部前台窗口，发送前再次确认窗口未变化；临时写入剪贴板后通过扫描码按下 Ctrl+V 并跨帧保持按键，然后恢复原剪贴板；非文本剪贴板或无法备份时退回 Unicode 按键事件。`src/platform/voice_overlay_window.rs` 会尽量移除 Windows 添加的原生边框，并设置不激活窗口样式。主窗口更新后显式请求子视口重绘，确保识别原文在 AI 请求尚未完成时先出现。资源放置和发布要求见 [assets/voice/README.md](../assets/voice/README.md)。

Whisper CLI 的计算线程数自动取可用逻辑处理器数与 8 的较小值（查询失败则用 4）。实测同一段 4.5 秒中文录音，4 线程约 1.18 秒、8 线程约 0.75 秒，文字相同；更激进的解码参数虽然略快，但会丢失标点，因此没有启用。启用耗时诊断时，日志中的 `asr_threads` 会记录实际线程数。

最终文字先按现有 `WordIndex` 在 key/value 两列搜索；命中时输入 key，多条命中默认输入第一条。设置项 `voice_auto_copy_first=false` 时，多条结果会交给现有查询框选择（手动选择沿用查询框的复制流程）。未命中时，仅在启用 `ai_translation`、配置目标语言和 DeepSeek API Key 后调用现有双向翻译，再输入译文，并异步保存到对应词库。窗口变化或输入被系统拒绝时跳过输入，避免写入错误窗口。设置项通过 `hotkeys.voice` 配置快捷键，默认 `ctrl+alt+v`；`voice_language` 默认 `auto`，可指定 Whisper 的识别语言。模型优先级为 medium → small → base → tiny。

耗时诊断默认关闭，不创建或续写日志。需要临时诊断时可用 `cargo run --features voice-metrics` 启用；`src/platform/voice_metrics.rs` 会向程序目录的 `logs/voice_metrics.jsonl` 追加 JSON Lines。`session_id` 和 `utterance_id` 用于关联同一次录音与句子；`since_session_ms` 是从该次录音开始算起的时间，`duration_ms` 是当前阶段耗时。重点看 `speech_end_estimated` → `vad_complete`（停顿判断）、`audio_segment.duration_ms`（实际送入模型的音频长度，非处理耗时）、`asr_completed`（本地 Whisper）、`overlay_original_drawn`（原文绘制回调已执行）、`lookup_completed`（词库）、`credential_load`、`ai_request_completed`（整个 AI 调用）及 `text_input`。`vad_complete.result=max_duration` 表示没有检测到停顿、达到 30 秒上限才提交；此时真实说话结束时间未知，不会填写 `finished.duration_ms`。正常停顿时该总耗时是估算的说话结束到输入或失败的时间；`overlay_hide_scheduled` 和 `overlay_hidden` 对应完成后的 1 秒停留时间。日志不记录用户文字、音频或密钥，`result` 只记录固定状态或模型文件名。

`voice_aion2_manual_paste` 默认 `false`。启用后，`VoiceInputTarget` 通过 Windows 进程快照比对当前前台窗口所属进程名；仅在 `Aion2.exe` 前台时跳过模拟按键，直接将结果保留在剪贴板供用户手动粘贴。正常输出路径的托盘状态会标明发送的是 Ctrl+V、Unicode，还是等待手动粘贴；`SendInput` 成功仅代表事件入队，不代表游戏已接收。

### 本地 OCR

按 OCR 快捷键后，程序会先隐藏窗口并截取鼠标所在显示器，再显示截图供框选。松开鼠标后选区不会消失，而是稍后自动识别并在选区旁预览译文；拖动选区内部可移动，拖动右下角斜线可缩放，调整后会重新识别。点击选区旁的「√」复制 OCR 原文并关闭整个框选界面，译文同时消失；点击「×」或按 `Esc` 可取消。截图只在本机内存中处理，不上传网络，也不修改词库。

设置项 `ocr_auto_translate` 默认 `true`。选区稳定 300 毫秒后在后台进行本地 OCR；开启设置项后再把识别文字发送给 DeepSeek，请求译成设置中的原始语言，原始语言为 `auto` 时使用简体中文 `cn`。译文直接画在同一截图窗口的选区下沿，缩小后的确认/取消按钮贴在选区右侧下方，不占用译文的空间；屏幕下方空间不足时译文移到选区上方。没有独立文本框或窗口。确认时仅复制 OCR 原文，关闭选区后译文一起消失；即使翻译失败也可复制原文。如果确认时本地识别还没完成，窗口先关闭，识别完成后再复制原文。关闭设置项时不请求翻译；没有 Key、凭据读取失败或翻译失败时，在选区旁显示原因。OCR 截图不会发送给 API；文字发送可能产生费用。此项独立于查询框的 `ai_translation` 开关，不会自动修改词库。

OCR 需要预先安装 [Tesseract OCR](https://github.com/tesseract-ocr/tesseract)。程序会自动寻找系统安装记录、常见安装目录、`PATH` 以及程序旁的 `tesseract.exe`；也可通过 `TESSERACT_EXE` 环境变量指定完整路径。设置页的快捷键模块会显示找到的程序和已安装语言包。

识别哪种文字，就需要相应的 [Tesseract 语言包](https://github.com/tesseract-ocr/tessdata_fast)：简体中文 `chi_sim`、繁体中文 `chi_tra`、韩文 `kor`、日文 `jpn`、英文 `eng`。只有英文包时，无法可靠识别中文或韩文。

安装语言包：
进入上方链接，打开所需的 `*.traineddata` 文件并点击「Download raw file」，把文件原名放到 `tesseract.exe` 旁边的 `tessdata` 文件夹。
例如本机安装在 `D:\Tesseract_OCR\tesseract.exe` 时，应保存为 `D:\Tesseract_OCR\tessdata\chi_sim.traineddata` 或 `D:\Tesseract_OCR\tessdata\kor.traineddata`。不要保存成 `.txt` 或 GitHub 网页。
之后在 PowerShell 执行 `& 'D:\Tesseract_OCR\tesseract.exe' --list-langs`，确认输出包含 `chi_sim`、`kor` 等语言代码，再重启 `to_words`。需要发布便携版时，也可将 `tesseract.exe` 和 `tessdata` 一起放在程序目录的 `assets/ocr` 下。
详细安装方法也可参考 [Tesseract 官方安装说明](https://github.com/tesseract-ocr/tessdoc/blob/main/Installation.md)。

## 查询和组合词条

1. 使用快捷键打开查询框。
2. 输入 key 或 value 中包含的文字。
3. 使用 `↑`、`↓` 选择结果。
4. 需要继续拼接时，按空格将当前选中结果加入组合；输入框清空后可继续搜索下一条。
5. 最后按一次 `Enter`，复制全部组合内容。默认同时关闭查询框。

查询同时匹配 key 和 value，并固定展示完整的 `key + value`。结果按照以下等级排序：

1. 完全匹配
2. 前缀匹配
3. 包含匹配

相同等级按照 key 排序。匹配到的文字会在 key 或 value 中高亮显示。

查询结果默认每页最多显示 8 条，可在设置中修改。使用 `←`、`→` 查看前后页。

### 组合内容

- 每次选择结果后，下方都会显示 `key（value）` 组合内容。
- 复制时按顺序拼接所有 key，以单个空格连接，不包含尾部空格。
- 空格只提交当前选中的匹配结果，暂不复制；没有匹配时保留当前查询文字供修改。
- 已有组合内容且输入框为空时，按一次 `Enter` 也可以复制并结束。
- 查询框为空时，按 `Backspace` 撤销最后一项。
- 按“清空组合内容”快捷键删除全部组合项，默认为 `Ctrl + Backspace`。
- 开启“Enter 复制后仍保持查询框打开”后，按 `Enter` 复制但不会关闭，可继续输入。

组合完成后，切换到目标程序并按 `Ctrl + V` 即可粘贴完整内容。

其他操作：

- 没有匹配结果时，连续按两次 `Enter` 清空当前查询文字。
- 启用 DeepSeek 后，无论本地词库是否已有匹配，都可以点击“使用 DeepSeek 翻译”，或按 `Ctrl + Enter` 主动翻译当前输入文字。
- 按 `Esc` 隐藏查询框。
- 点击右侧 `×` 彻底退出程序。
- 查询框可以拖动。
- 通过 `Alt + Tab` 切回后会自动恢复输入焦点。

查询框会依次通过原生文本光标、Windows UI Automation、MSAA caret 和输入法候选坐标获取位置，兼容记事本、IDEA、Electron 应用和常见浏览器输入框，并优先显示在光标下方；下方空间不足时自动显示到光标上方。若应用或游戏未提供可访问的文本光标位置，则回退到前台窗口所在显示器的左下方；无法取得前台窗口时使用鼠标所在显示器。

## 词库格式

用户词库统一存放在程序目录下的 `word_libraries` 文件夹。未选择目标语言时使用 `word_libraries/user_words.json`。key 是复制内容，value 是对应的说明或翻译：

```json
{
  "안녕하세요": "你好",
  "감사합니다": "谢谢"
}
```

输入“你好”可以找到 `안녕하세요`；输入 key 中的韩文也可以找到同一条完整记录。

### 多语言词库

设置页面分别选择“原始语言”和“目标语言”。原始语言默认“自动检测”，会根据查询框输入切换相应词库；短且无法可靠判断的文字会沿用当前词库。

- 未选择目标语言：使用 `word_libraries/user_words.json`
- 简体中文 → 韩语：使用 `word_libraries/user_words_cn_ko.json`，兼容已有词库
- 英语 → 韩语：使用 `word_libraries/user_words_en_ko.json`

对应文件不存在时会在 `word_libraries` 中自动创建空 JSON 词库（`{}`），已有文件不会被覆盖。自动检测对极短或混合语言输入可能不准确；需要固定词库时可手动选择原始语言。

兼容旧版本：程序会把根目录中现有的 `user_words*.json` 复制到 `word_libraries`，不会删除或改写根目录中的原文件。

## 词库编辑器

在设置页面点击“编辑词库”即可在当前设置窗口中进入词库编辑页面；点击“返回设置”可回到设置页。

结构化表格模式支持：

- 自适应 key/value 两列表格
- 新增、修改、复制和删除词条
- 新增词条自动插入第一行
- 根据 key 或 value 筛选
- 每页最多展示 50 条数据
- 批量导入 JSON
- 批量导出格式化后的 JSON 文件

批量导入和批量导出也会在当前窗口内切换，并可返回词库编辑页面，不会再打开新的系统窗口。

批量导出时会先询问文件名，自动补充 `.json` 扩展名，并将文件保存到 `to_words.exe` 所在目录。

保存词库时会：

- 检查 JSON 格式
- 自动格式化 JSON
- 删除空 key
- 删除空 value
- 去除重复 key，重复时保留最后一项
- 保存后立即重新加载词库

高级 JSON 编辑模式支持：

- 直接编辑 JSON 原文
- 搜索中文或韩文
- 按 `Enter` 查找下一处
- 检查 JSON 格式
- 显示错误所在的行和列
- 自动滚动到错误位置
- 使用浅红色高亮错误行

## 合并个人词库

为了避免发布新版本时覆盖用户自己的词条，可以把个人词库放在 `word_libraries` 目录中，例如：

```text
user_words.local.json
user_words_extra.json
user_words (1).json
```

在设置页面点击“合并本地词库”后，会进入词库选择页面：

1. 当前设置的原始语言与目标语言对应词库作为合并目标；原始语言为“自动检测”时使用当前已识别的语言。
2. 勾选一个或多个来源词库。
3. 点击“合并所选词库”。

程序只合并用户勾选的文件：

- 所有源文件验证通过后才会写入主词库
- 完全相同的 key/value 自动跳过
- key 相同但 value 不同时，以用户导入文件为准
- 合并结果自动去重和格式化
- 源文件不会被删除，可以在版本更新后再次合并
- 支持全部选择、清空选择和刷新目录
- 合并完成后立即重新加载，无需重启程序

## 设置页面

设置页面可以调整：

- 查询框和结果区域的颜色、透明度、尺寸、字体和圆角
- 原始语言、目标语言和对应用户词库
- 查询结果每页最大数量
- 查询框、设置页面、本地 OCR 和清空组合内容快捷键
- 查询框是否持续保持打开
- 返回查询框后是否自动清空
- 选中状态外观
- DeepSeek AI 翻译开关和用户自己的 API Key

快捷键框点击后直接按下新的组合键。数值项点击后使用鼠标滚轮调整，点击“保存并应用”后立即生效。

配置保存在 `ui_config.json` 中。

### DeepSeek AI 补充翻译

1. 在设置中选择目标语言，启用“AI 翻译”，填写自己的 DeepSeek API Key，然后点击“保存并应用”。Key 存在当前 Windows 用户的凭据管理器中，不写入配置文件。
2. 查询框优先搜索本地词库。需要自行翻译当前输入文字时，即使已有本地匹配，也可以点击“使用 DeepSeek 翻译”或按 `Ctrl + Enter`；查询框只有主动操作才会向 DeepSeek 发送请求。OCR 的自动翻译则由独立开关控制；两种请求都可能产生 API 费用。
3. 等待 AI 译文出现后，按 `Enter` 或点击译文即可加入组合并复制。输入原始语言时翻译成目标语言；输入目标语言时反向翻译成原始语言。若原始语言设为“自动检测”，反向翻译默认使用简体中文。默认还会自动保存到对应语言词库：词库中的 key 始终是目标语言文字，value 始终是原始语言文字。
4. 若不希望自动收录，可以在设置中取消勾选“确认 AI 译文后自动保存到当前语言词库”；之后仍可点击查询框中的“将刚才的 AI 译文保存到词库”手动收录。保存前会检查重复 key，不会生成 `.bak` 备份文件。

未选择目标语言、词库加载失败或未配置 Key 时无法发起请求。AI 请求在后台执行，网络错误不会阻塞查询框；切换输入后过期的结果不会进入当前查询。普通 `Enter` 的“双击清空无匹配文字”行为仍可使用。

## 系统托盘

右键系统托盘图标可以：

- 打开查询
- 打开设置
- 重新加载词库
- 退出程序

## 性能优化

运行时词库使用按 key 排序的 `Vec<WordEntry>`，查询结果只保存轻量的词条编号。程序还使用输入增长增量过滤和 key/value 双字段字符二元索引，并且只读取和绘制当前结果页需要的数据，适合逐渐增大的用户词库。

## 构建发布版本

### Windows 本地语音识别（实验版）

普通构建仅启用 `whisper.cpp`；Windows 本地识别实验代码保留在仓库工作区，通过 `windows-speech` Cargo feature 才会编译和显示设置选项，`packaging/windows_speech/build.ps1` 会显式启用该 feature。两种引擎共用现有录音、停顿检测、词库优先查询和结果输入流程。语音 AI 译文会异步写入对应的 `word_libraries/user_words_<原始语言>_<目标语言>.json`；相同 key/value 不重复写，冲突 key 不覆盖原词条。Whisper 仍是默认引擎。

Windows 路线使用 `windows_speech_bridge/` 的 .NET 8 桥接程序和微软实验版 `Microsoft.Windows.AI.Speech`。当前文档没有公开的识别语言设置方式；对日语等语言可能返回英语译文，因此用户需要在自己的机器上验证结果。桥接程序要求 Windows 11 24H2+、WinAppSDK 实验版语音 API、已安装的本地语音模型，以及带 `systemAIModels` 权限的 MSIX 包身份。桥接程序使用 Windows App SDK 自包含发布，避免另外安装匹配的实验版 Windows App Runtime；这与 .NET 自包含发布是两个不同的选项。普通 `cargo run` 没有包身份，不能直接测试 Windows 选项。

开发/发布准备：

1. 安装 .NET 8 或更新版本的 SDK（本项目已用 10.0.401 编译桥接程序）、Windows SDK，并取得与 `packaging/windows_speech/AppxManifest.xml` 中 `Publisher` 一致的代码签名证书。生产发布应使用受信任的生产证书，不要把 `.pfx` 私钥提交到仓库。
2. 运行 `powershell -ExecutionPolicy Bypass -File packaging/windows_speech/build.ps1`。脚本会构建 Rust 主程序和 .NET 桥接程序，把它们与配置/模型资源放入一个新的 `target/to_words_msix_stage_<时间>` 目录，将身份嵌入两个 EXE，并生成**未签名**的稀疏 MSIX 身份包。它不会复制你的词库，也不会安装证书或自动注册 MSIX。发布时保留桥接程序发布输出的**全部文件**，不仅是 `.exe`。
3. 用 `signtool.exe` 对 `.msix` 签名。清单中的包名、Publisher、Application Id 必须和两个 EXE 内嵌清单完全一致。每次升级身份包应递增清单 Version。已有用户的 `word_libraries` 要单独保留并合并，不要用空目录覆盖。
4. 自签名证书仅限开发测试。安装 MSIX 前，须经管理员同意将**公钥证书**导入 `Cert:\LocalMachine\TrustedPeople`；仅导入 `Cert:\CurrentUser\Root` 或 `Cert:\CurrentUser\TrustedPeople` 仍可能报 `0x800B0109`。这项信任会影响本机所有用户，测试完应移除对应证书。生产发布应使用受信任的正式签名方式，不应要求用户信任开发证书。
5. 将签名后的身份包和发布目录一起部署，并以发布目录作为外部位置注册：`Add-AppxPackage -Path <身份包绝对路径> -ExternalLocation <发布目录绝对路径>`。这是微软的 *packaging with external location*（稀疏 MSIX）方案：保留程序目录和用户词库的现有行为，但身份包本身**不是**包含所有程序文件的独立安装器。不要把外部位置放在普通用户不可写的 `Program Files` 中，除非另行迁移配置和词库存储。
6. 启动该发布目录中的程序，在设置里选 Windows 引擎，点击“准备 Windows 语音模型”并确认。CPU 设备可能通过 Windows Update 首次下载模型，等待就绪后再尝试语音快捷键。

若本机只有 .NET Runtime、没有 .NET SDK，`build.ps1` 会在构建前停止。未签名的 MSIX 身份包不能直接安装；签名与实机识别验证是发布前的必要步骤。

```powershell
cargo build --release
```

生成的程序位于：

```text
target\release\to_words.exe
```

发布时请将程序和词库放在同一目录，建议同时携带配置文件：

```text
to_words.exe
ui_config.json
word_libraries\
  user_words.json
```

如果使用多语言词库，请把相应文件放入 `word_libraries`，例如韩语对应 `word_libraries/user_words_cn_ko.json`。

如果缺少 `ui_config.json`，程序会使用默认设置。
