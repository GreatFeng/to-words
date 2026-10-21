# 项目结构

```text
hotkey_word_match/
├─ assets/                 # 字体、图标、OCR 与语音模型等资源
│  └─ i18n/                # 设置界面的英、韩、日文翻译表
├─ docs/                   # 架构和开发文档
├─ frontend/               # React、TailwindCSS 页面与 Tauri 调用封装
├─ capabilities/           # Tauri 窗口权限
├─ tauri.conf.json          # 桌面窗口和前端资源配置
├─ src/
│  ├─ ai/                  # DeepSeek 请求与候选译文解析
│  ├─ domain/              # 查询索引、SQLite 持久化、词库合并及 AI 词条保存
│  ├─ platform/            # 系统托盘、凭据、OCR、麦克风与本地语音识别
│  │  └─ caret_position.rs  # Windows 多种文字光标定位方式
│  ├─ i18n.rs              # 界面语言选择、翻译查找与动态文字插值
│  ├─ web_app.rs           # Tauri 状态、窗口、托盘和快捷键调度
│  ├─ web_app/             # 词库、OCR 和语音命令/工作流
│  ├─ web_config.rs        # 设置模型与持久化接口
│  └─ main.rs              # 程序入口和模块装配
├─ tests/                  # 后续跨模块集成测试
├─ to_words.db             # 运行时词库、设置与待审核语音缓存
└─ word_libraries/         # 旧版 JSON 迁移与手动导入导出
```

## 依赖方向

- `main` 只负责启动 React/Tauri 应用。
- React 页面通过 Tauri 命令调用 `web_app`，后者复用 `domain` 与 `platform`。
- `frontend/src/pages` 负责展示和收集交互，业务规则优先放入 `domain`。
- `domain` 尽量不依赖界面框架或 Windows API，以便独立测试。
- `platform` 封装托盘、窗口、光标定位等系统差异。

新增功能时，先判断它属于界面、业务规则还是系统能力，避免再次把逻辑集中到入口文件。
