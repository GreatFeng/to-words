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

快捷键均可在设置页面重新录制。键盘组合仍交由 `global-hotkey` 注册；鼠标左、右、中键与标准 X1/X2 侧键由 `src/platform/mouse_hotkey.rs` 的 Windows 低级鼠标钩子观察。`src/platform/extra_mouse_buttons.rs` 以后台非独占 DirectInput 读取 `DIMOUSESTATE2` 的第 6～8 键，前五键不重复派发；此路径仅在驱动真正公开这些逻辑按键时有效。`src/domain/shortcut.rs` 统一解析与去重。监听不拦截原始点击，按键录制会忽略打开录制按钮的那次点击。G HUB 若把不同物理侧键都映射为 X1/X2，则应在驱动中改成不同键盘快捷键后录制，程序无法从相同逻辑事件还原物理按键。

G502 LIGHTSPEED 的 11 个可编程功能不等于 Windows 上报 11 个鼠标按键。DPI/G-Shift 等无独立鼠标事件的功能需通过 G HUB 映射为不同键盘快捷键；录制路径支持 F13～F24，并保留按键松开事件以适配按住说话。设置页有折叠说明，翻译键写在 `assets/i18n`；不要假定 `Mouse6`～`Mouse8` 在 G502 上一定由驱动公开。

### 设置界面语言

`ui_language` 独立于 `source_language` 和 `translation_language`，默认 `zh-CN`，还支持 `en`、`ko`、`ja`。设置首页、词库编辑与合并页面共用 `src/i18n.rs` 的本地化入口；译文存放在 `assets/i18n/*.json` 并编译进程序。新增界面文字时以中文原文为键，同步补齐三份译文，动态文字使用命名占位符（如 `{count}`）。词库内容与文件命名不会随界面语言变化。

### 本地语音识别

语音模块见 `src/platform/voice.rs`，使用 Windows `waveIn` 以 16 kHz 单声道录音，默认调用随程序发布的 SenseVoice CLI、SenseVoice GGUF 模型与 FSMN VAD；也可选择 `whisper-cli.exe` 和多语言 GGML 模型。录音/停顿检测与语音识别使用独立线程，上一句识别时仍能检测下一句，识别任务按语句顺序处理；每 100 毫秒分析一帧音量，开头约 0.4 秒估计背景噪音，说话中根据语音峰值动态判断停顿，检测到约 0.6 秒停顿后只识别完整句子一次，不再每四秒重复启动模型。设置项 `voice_keep_input` 默认 `false`；开启后，每句自动提交并继续监听，直到再次按快捷键结束。语音活动检测仍是音量阈值启发式算法，持续人声或音乐等复杂噪声可能需要手动停止。悬浮文字由 `src/components/voice_overlay.rs` 的独立透明、置顶、鼠标穿透视口绘制，始终位于屏幕底部；最终结果显示约 1 秒后消失，即使保持语音输入也不会一直占据屏幕，下一句开始时清空旧文字。`src/platform/voice_output.rs` 在每条结果产生时捕获当前外部前台窗口，发送前再次确认窗口未变化；临时写入剪贴板后通过扫描码按下 Ctrl+V 并跨帧保持按键，然后恢复原剪贴板；非文本剪贴板或无法备份时退回 Unicode 按键事件。`src/platform/voice_overlay_window.rs` 会尽量移除 Windows 添加的原生边框，并设置不激活窗口样式。主窗口更新后显式请求子视口重绘，确保识别原文在 AI 请求尚未完成时先出现。资源放置和发布要求见 [assets/voice/README.md](../assets/voice/README.md)。

Whisper CLI 的计算线程数自动取可用逻辑处理器数与 8 的较小值（查询失败则用 4）。实测同一段 4.5 秒中文录音，4 线程约 1.18 秒、8 线程约 0.75 秒，文字相同；更激进的解码参数虽然略快，但会丢失标点，因此没有启用。启用耗时诊断时，日志中的 `asr_threads` 会记录实际线程数。

`voice_silence_seconds` 默认 `0.6`，读取配置后限制在 `0.2..=3.0`，按 16 kHz 采样数控制自动提交。`voice_hold_to_talk` 默认 `false`：启用后全局语音快捷键的 Pressed 开始录音，Released 停止；按住期间不按静音阈值自动提交，最后一段在松开后交给识别线程，并短暂等待快捷键修饰键松开再输入结果。此模式优先于 `voice_keep_input`。`ai_polite_mode` 默认 `false`，对查询框、语音及 OCR 的 AI 翻译结果添加按目标语言选择的礼貌表达提示；正向、反向均适用，不改写本地词库匹配结果。旧配置字段 `ai_korean_honorific` 作为反序列化别名继续读取。

最终文字先按现有 `WordIndex` 在 key/value 两列搜索；命中时输入 key，多条命中默认输入第一条。设置项 `voice_auto_copy_first=false` 时，多条结果会交给现有查询框选择（手动选择沿用查询框的复制流程）。未命中时，仅在启用 `ai_translation`、配置目标语言和 DeepSeek API Key 后调用现有双向翻译，再立即输入译文；入库则交由后台缓存审核。窗口变化或输入被系统拒绝时跳过输入，避免写入错误窗口。设置项通过 `hotkeys.voice` 配置快捷键，默认 `ctrl+alt+v`；`voice_language` 默认 `auto`，可指定 Whisper 的识别语言。模型优先级为 medium → small → base → tiny。

语音自动入库由 `src/domain/voice_word_cache.rs` 管理，待审核记录持久化到程序目录的 `to_words.db`（被 Git 忽略，不含录音或 API Key），重启后继续审核；旧版 `voice_word_cache.json` 只读迁入。空串、几乎全符号、异常连续重复字符、已知无语音/低可信标记以及两分钟内相同识别文本直接跳过；模型若仅返回普通文本而不提供可信度，不能额外推断数值置信度。缓存满 20 条或距上次审核约 1 小时触发，单次最多处理 100 条，AI 每批审核 10 条并使用同一录音会话内相邻语句作上下文。`normal` 写入对应词库后清缓存，`error` 直接移除，`uncertain` 留存，只有相邻上下文变化才重新审核；无新依据 7 天后清理。网络/凭据/词库写入失败时保留待处理记录，下次再试；已有冲突 key 不被覆盖。审核请求只发送识别文本、已有译文和相邻语句，可能产生额外 API 费用。查询框手动翻译及 OCR 路径不经过此缓存。

显式配置的 `source_language` 与 `translation_language` 相同时，语音路径在词库查询前直接输出识别原文，不读取 API Key、不调用 AI、不保存词条；OCR 路径仍复制识别原文，但不启动翻译。`auto` 不被视为与目标语言相同。

耗时诊断默认关闭，不创建或续写日志。需要临时诊断时可用 `cargo run --features voice-metrics` 启用；`src/platform/voice_metrics.rs` 会向程序目录的 `logs/voice_metrics.jsonl` 追加 JSON Lines。`session_id` 和 `utterance_id` 用于关联同一次录音与句子；`since_session_ms` 是从该次录音开始算起的时间，`duration_ms` 是当前阶段耗时。重点看 `speech_end_estimated` → `vad_complete`（停顿判断）、`audio_segment.duration_ms`（实际送入模型的音频长度，非处理耗时）、`asr_completed`（本地 Whisper）、`overlay_original_drawn`（原文绘制回调已执行）、`lookup_completed`（词库）、`credential_load`、`ai_request_completed`（整个 AI 调用）及 `text_input`。`vad_complete.result=max_duration` 表示没有检测到停顿、达到 30 秒上限才提交；此时真实说话结束时间未知，不会填写 `finished.duration_ms`。正常停顿时该总耗时是估算的说话结束到输入或失败的时间；`overlay_hide_scheduled` 和 `overlay_hidden` 对应完成后的 1 秒停留时间。日志不记录用户文字、音频或密钥，`result` 只记录固定状态或模型文件名。

`voice_aion2_manual_paste` 默认 `false`。启用后，`VoiceInputTarget` 通过 Windows 进程快照比对当前前台窗口所属进程名；仅在 `Aion2.exe` 前台时跳过模拟按键，直接将结果保留在剪贴板供用户手动粘贴。正常输出路径的托盘状态会标明发送的是 Ctrl+V、Unicode，还是等待手动粘贴；`SendInput` 成功仅代表事件入队，不代表游戏已接收。

### 本地 OCR

按 OCR 快捷键后，程序会先隐藏窗口并截取鼠标所在显示器，再显示截图供框选。松开鼠标后选区不会消失，而是稍后自动识别并在选区旁预览译文；拖动选区内部可移动，拖动右下角斜线可缩放，调整后会重新识别。点击选区旁的「√」复制 OCR 原文并关闭整个框选界面，译文同时消失；点击「×」或按 `Esc` 可取消。截图只在本机内存中处理，不上传网络，也不修改词库。

设置项 `ocr_auto_translate` 默认 `true`。选区稳定 300 毫秒后在后台进行本地 OCR；开启设置项后再把识别文字发送给 DeepSeek，请求译成设置中的原始语言，原始语言为 `auto` 时使用简体中文 `cn`。译文直接画在同一截图窗口的选区下沿，缩小后的确认/取消按钮贴在选区右侧下方，不占用译文的空间；屏幕下方空间不足时译文移到选区上方。没有独立文本框或窗口。确认时仅复制 OCR 原文，关闭选区后译文一起消失；即使翻译失败也可复制原文。如果确认时本地识别还没完成，窗口先关闭，识别完成后再复制原文。关闭设置项时不请求翻译；没有 Key、凭据读取失败或翻译失败时，在选区旁显示原因。OCR 截图不会发送给 API；文字发送可能产生费用。此项独立于查询框的 `ai_translation` 开关，不会自动修改词库。

OCR 需要预先安装 [Tesseract OCR](https://github.com/tesseract-ocr/tesseract)。程序会自动寻找系统安装记录、常见安装目录、`PATH` 以及程序旁的 `tesseract.exe`；也可通过 `TESSERACT_EXE` 环境变量指定完整路径。设置页的快捷键模块会显示找到的程序和已安装语言包。

识别哪种文字，就需要相应的 [Tesseract 语言包](https://github.com/tesseract-ocr/tessdata_fast)：简体中文 `chi_sim`、繁体中文 `chi_tra`、韩文 `kor`、日文 `jpn`、英文 `eng`。只有英文包时，无法可靠识别中文或韩文。

`src/platform/ocr.rs` 会优先将已配置的目标语言一对一映射到 Tesseract 模型，不再把中、韩、英、日模型拼接识别；目标语言包缺失时明确报错。未选择目标语言时才回退到显式原始语言，最后回退到单个已安装模型。选区在内存中依据边缘亮度判断是否为浅字深底：必要时反色为深字浅底，再给小选区加 10 像素白边；只有高度小于 24 像素时才放大两倍，避免普通小字无条件放大后笔画失真。高度不超过 64 像素的横向选区使用 `--psm 7`，其余保留 `--psm 6`。这些处理参考 [Tesseract 图像质量指南](https://tesseract-ocr.github.io/tessdoc/ImproveQuality.html)，不会更改截图原件或强行改写识别文字。

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

用户词库保存在程序目录下的 `to_words.db`。`word_libraries` 中的 JSON 文件仅供旧版首次迁移和手动导入/导出。未选择目标语言时使用数据库中名为 `user_words.json` 的词库。key 是复制内容，value 是对应的说明或翻译：

```json
{
  "안녕하세요": "你好",
  "감사합니다": "谢谢"
}
```

输入“你好”可以找到 `안녕하세요`；输入 key 中的韩文也可以找到同一条完整记录。

### 多语言词库

设置页面分别选择“原始语言”和“目标语言”。原始语言默认“自动检测”，会根据查询框输入切换相应词库；短且无法可靠判断的文字会沿用当前词库。

- 未选择目标语言：使用数据库词库 `user_words.json`
- 简体中文 → 韩语：使用数据库词库 `user_words_cn_ko.json`，兼容已有词库
- 英语 → 韩语：使用数据库词库 `user_words_en_ko.json`

对应词库不存在时使用空词库，首次保存后在数据库中创建记录，不再生成 JSON 文件。自动检测对极短或混合语言输入可能不准确；需要固定词库时可手动选择原始语言。

兼容旧版本：首次建库会只读导入 `word_libraries` 和程序根目录中的 `user_words*.json`、`ui_config.json` 与待审核语音缓存；原文件不会删除或改写。旧 JSON 没有逐条时间戳，迁入时用文件修改时间作为入库时间。数据库用 `word_entries`、`app_settings`、`voice_cache_state` 和 `app_meta` 分表保存；词条还有读音、来源、更新时间及质量状态。明显错误词条标记为 `quarantined` 并从查询/编辑结果排除，原始 JSON 仍保留。可运行 `to_words.exe --migrate-data` 查看各词库迁入和隔离数量。

`word_entries` 以 `id` 为内部编号，`library_name` 标识语言词库；`key_text` 是最终输出的译文，`value_text` 是原文或说明，`pronunciation` 是 key 的读音/罗马化（旧词条为空，可在编辑器中补充）。`created_at` 与 `updated_at` 是毫秒时间戳，`origin` 区分手工、AI 和旧版导入；`quality_status`/`quality_reason` 记录隔离结果。同一词库内的 key 唯一。`word_libraries` 表还保留没有词条的空语言词库。词库编辑器按 `created_at DESC, id DESC` 展示。

## 词库编辑器

在设置页面点击“编辑词库”即可在当前设置窗口中进入词库编辑页面；点击“返回设置”可回到设置页。

点击“编辑词库”还会向语音缓存后台线程发送一次强制审核请求，绕过一小时／20 条门槛，但网络审核仍在后台完成。审核通过后刷新未修改的编辑器；若已有未保存的草稿，保留草稿，并在保存时合并审核期间新增到磁盘的 key。词库写入使用进程内互斥锁，避免语音审核和编辑器保存交错覆盖。

结构化表格模式支持：

- 自适应 key/value/读音表格，默认最新入库在前（时间戳不在表格中显示）
- 新增、修改、复制和删除词条
- 新增词条自动插入第一行
- 根据 key 或 value 筛选
- 每页最多展示 50 条数据
- 批量导入 JSON
- 批量导出格式化后的 JSON 文件

批量导入和批量导出也会在当前窗口内切换，并可返回词库编辑页面，不会再打开新的系统窗口。

批量导出时会先询问文件名，自动补充 `.json` 扩展名，并将文件保存到程序目录下的 `word_libraries` 文件夹。

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

配置保存在 `to_words.db` 的 `app_settings` 表中；API Key 仍在 Windows 凭据管理器。

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

程序提供 SenseVoice（默认）和 `whisper.cpp` 两种本地识别引擎，共用录音、停顿检测、词库优先查询和结果输入流程。语音 AI 译文在缓存审核通过后写入 `to_words.db` 对应语言词库；相同 key/value 不重复写，冲突 key 不覆盖原词条。

```powershell
cargo build --release
```

生成的程序位于：

```text
target\release\to_words.exe
```

升级时请将程序和用户数据库放在同一目录：

```text
to_words.exe
to_words.db
word_libraries\  # 可选：待合并或待导入的旧 JSON
```

首次安装若没有 `to_words.db`，程序会创建空数据库并使用默认设置；从旧版升级时把原有 JSON 文件一并放入程序目录，由程序首次启动迁入。迁移完成后备份 `to_words.db` 即可保留词库、设置和待审核语音缓存。
