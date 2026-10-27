//! Immediate voice-command operations. Keep these on the same entry points as
//! their hotkeys so command mode does not invent a second execution path.

use super::*;
use settings_command::CommandAction;

pub(super) fn execute_command_action(
    app: &AppHandle,
    action: CommandAction,
) -> Result<String, String> {
    match action {
        CommandAction::OpenOcr => {
            // A visible voice overlay would otherwise be part of the screenshot.
            hide_voice(app);
            std::thread::sleep(Duration::from_millis(70));
            open_ocr(app)?;
            Ok("已打开 OCR 截图".into())
        }
        CommandAction::OpenQuery => {
            open_query(app);
            Ok("已打开查询框".into())
        }
        CommandAction::OpenSettings => {
            open_settings(app, SettingsOrigin::Keyboard);
            Ok("已打开设置面板".into())
        }
        CommandAction::MicTts(desired) => {
            let enabled = change_mic_tts(app, desired, None)?;
            Ok(format!(
                "已{}自动麦克风发音",
                if enabled { "开启" } else { "关闭" }
            ))
        }
        CommandAction::SystemAudio(desired) => {
            let enabled = change_system_audio(app, desired)?;
            Ok(format!(
                "已{}系统声音字幕与翻译",
                if enabled { "开启" } else { "关闭" }
            ))
        }
        CommandAction::TranslateText(text) => {
            prepare_translation(app)?;
            start_translation(app, text);
            Ok("已打开文字翻译，正在生成译文".into())
        }
        CommandAction::TranslateSelection => {
            if !SELECTED_TEXT_TRANSLATION_ACTIVE {
                return Err("选中文字翻译已暂时停用".into());
            }
            prepare_translation(app)?;
            let text = selected_text::read_selected_text()
                .ok_or("未读取到选中文字，请先在其他程序中选中文字")?;
            start_translation(app, text);
            Ok("已打开选中文字翻译，正在生成译文".into())
        }
        CommandAction::StartVoice => Err("语音识别切换由录音队列处理".into()),
    }
}

fn prepare_translation(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<WebState>();
    let config = state.config.lock().map_err(error)?;
    if config.translation_language.is_empty() {
        return Err("请先在设置中选择目标语言".into());
    }
    if config.source_language != config.translation_language
        && !credentials::load_key()
            .map_err(error)?
            .is_some_and(|key| !key.trim().is_empty())
    {
        return Err("请先在设置中填写 DeepSeek API Key".into());
    }
    Ok(())
}
