# 项目结构

```text
hotkey_word_match/
├─ assets/                 # 字体、图标、OCR 与语音模型等资源
│  └─ i18n/                # 设置界面的英、韩、日文翻译表
├─ docs/                   # 架构和开发文档
├─ src/
│  ├─ app/                 # 应用状态、窗口生命周期与功能调度
│  ├─ ai/                  # DeepSeek 请求与候选译文解析
│  ├─ components/          # 界面组件
│  │  ├─ settings/         # 设置面板
│  │  └─ word_library/     # 词库编辑、导入导出和合并界面
│  ├─ domain/              # 查询索引、连续组合、词库合并及 AI 词条保存
│  ├─ platform/            # 系统托盘、凭据、OCR、麦克风与本地语音识别
│  ├─ i18n.rs              # 界面语言选择、翻译查找与动态文字插值
│  ├─ ui/                  # 颜色、间距等统一视觉规范
│  └─ main.rs              # 程序入口和模块装配
├─ tests/                  # 后续跨模块集成测试
└─ word_libraries/         # 用户词库数据
```

## 依赖方向

- `main` 只负责装配模块并启动应用。
- `app` 可以调用 `components`、`domain`、`platform` 和 `ui`。
- `components` 负责展示和收集交互，业务规则优先放入 `domain`。
- `domain` 尽量不依赖 egui 或 Windows API，以便独立测试。
- `platform` 封装托盘、窗口、光标定位等系统差异。

新增功能时，先判断它属于界面、业务规则还是系统能力，避免再次把逻辑集中到入口文件。
