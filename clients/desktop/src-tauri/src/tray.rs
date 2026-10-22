//! The menu bar icon: Open cos, how many things wait on you, and Quit.

use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Wry};

const OPEN: &str = "open";
const WAITING: &str = "waiting";
const QUIT: &str = "quit";

/// A monochrome template image, so macOS tints it for light and dark menu bars.
const ICON: &[u8] = include_bytes!("../icons/tray.png");

pub(crate) struct Tray {
    waiting: Mutex<MenuItem<Wry>>,
}

impl Tray {
    pub(crate) fn create(app: &AppHandle) -> tauri::Result<Self> {
        let open = MenuItem::with_id(app, OPEN, "Open cos", true, None::<&str>)?;
        let waiting = MenuItem::with_id(app, WAITING, waiting_label(0), true, None::<&str>)?;
        let quit = MenuItem::with_id(app, QUIT, "Quit cos", true, None::<&str>)?;
        let separator = PredefinedMenuItem::separator(app)?;
        let menu = Menu::with_items(app, &[&open, &waiting, &separator, &quit])?;

        TrayIconBuilder::with_id("cos")
            .icon(Image::from_bytes(ICON)?)
            .icon_as_template(true)
            .tooltip("cos")
            .menu(&menu)
            .show_menu_on_left_click(true)
            .on_menu_event(|app, event| match event.id().as_ref() {
                OPEN | WAITING => crate::show_main_window(app),
                QUIT => app.exit(0),
                _ => {}
            })
            .build(app)?;

        Ok(Self {
            waiting: Mutex::new(waiting),
        })
    }

    pub(crate) fn set_waiting(&self, count: u32) -> tauri::Result<()> {
        let waiting = self
            .waiting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        waiting.set_text(waiting_label(count))
    }
}

fn waiting_label(count: u32) -> String {
    match count {
        0 => "Nothing waiting on you".to_owned(),
        count => format!("{count} waiting on you"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_the_waiting_line() {
        assert_eq!(waiting_label(0), "Nothing waiting on you");
        assert_eq!(waiting_label(1), "1 waiting on you");
        assert_eq!(waiting_label(12), "12 waiting on you");
    }
}
