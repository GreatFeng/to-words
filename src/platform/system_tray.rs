//! 系统托盘适配器。
//!
//! `TrayAction` 将原生菜单事件转换为应用可以消费的动作；Windows 实现负责创建
//! 托盘图标、菜单和轮询事件，非 Windows 实现提供相同接口的空适配，确保工程
//! 可以进行跨平台编译检查。

#[cfg(target_os = "windows")]
const TRAY_ICON_SIZE: u32 = 32;

#[cfg(target_os = "windows")]
const TRAY_ICON_RGBA: &[u8; (TRAY_ICON_SIZE * TRAY_ICON_SIZE * 4) as usize] =
    include_bytes!("../../assets/icons/to_words_tray_32.rgba");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrayAction {
    OpenQuery,
    OpenSettings,
    ReloadWords,
    Exit,
}

#[cfg(target_os = "windows")]
pub(crate) struct SystemTray {
    tray: tray_icon::TrayIcon,
    open_query: tray_icon::menu::MenuId,
    open_settings: tray_icon::menu::MenuId,
    reload_words: tray_icon::menu::MenuId,
    exit: tray_icon::menu::MenuId,
}

#[cfg(target_os = "windows")]
impl SystemTray {
    pub(crate) fn new() -> anyhow::Result<Self> {
        use anyhow::Context;
        use tray_icon::{
            TrayIconBuilder,
            menu::{Menu, MenuItem, PredefinedMenuItem},
        };

        let menu = Menu::new();
        let open_query = MenuItem::new("打开查询", true, None);
        let open_settings = MenuItem::new("设置", true, None);
        let reload_words = MenuItem::new("重新加载词库", true, None);
        let separator = PredefinedMenuItem::separator();
        let exit = MenuItem::new("退出", true, None);
        menu.append_items(&[
            &open_query,
            &open_settings,
            &reload_words,
            &separator,
            &exit,
        ])
        .context("无法创建系统托盘菜单")?;

        let tray = TrayIconBuilder::new()
            .with_tooltip("to_words 已运行")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_icon(create_icon()?)
            .build()
            .context("无法创建系统托盘图标")?;

        Ok(Self {
            tray,
            open_query: open_query.id().clone(),
            open_settings: open_settings.id().clone(),
            reload_words: reload_words.id().clone(),
            exit: exit.id().clone(),
        })
    }

    pub(crate) fn poll_action(&self) -> Option<TrayAction> {
        let event = tray_icon::menu::MenuEvent::receiver().try_recv().ok()?;
        if event.id == self.open_query {
            Some(TrayAction::OpenQuery)
        } else if event.id == self.open_settings {
            Some(TrayAction::OpenSettings)
        } else if event.id == self.reload_words {
            Some(TrayAction::ReloadWords)
        } else if event.id == self.exit {
            Some(TrayAction::Exit)
        } else {
            None
        }
    }

    pub(crate) fn set_status(&self, status: &str) {
        let _ = self
            .tray
            .set_tooltip(Some(format!("hotkey_word_match - {status}")));
    }
}

#[cfg(target_os = "windows")]
fn create_icon() -> anyhow::Result<tray_icon::Icon> {
    use anyhow::Context;

    tray_icon::Icon::from_rgba(TRAY_ICON_RGBA.to_vec(), TRAY_ICON_SIZE, TRAY_ICON_SIZE)
        .context("无法加载系统托盘图标资源")
}

#[cfg(not(target_os = "windows"))]
pub(crate) struct SystemTray;

#[cfg(not(target_os = "windows"))]
impl SystemTray {
    pub(crate) fn new() -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub(crate) fn poll_action(&self) -> Option<TrayAction> {
        None
    }

    pub(crate) fn set_status(&self, _status: &str) {}
}
