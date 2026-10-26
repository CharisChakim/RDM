//! Tray icon: left click opens the window, the menu has Open and Quit.

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// `open` and `quit` run on the UI thread.
pub fn create(open: fn(), quit: fn()) -> TrayIcon {
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id("open", "Open RDM", true, None),
        &MenuItem::with_id("quit", "Quit", true, None),
    ])
    .expect("tray menu");
    TrayIconEvent::set_event_handler(Some(move |e| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            let _ = slint::invoke_from_event_loop(open);
        }
    }));
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| match e.id.0.as_str() {
        "open" => {
            let _ = slint::invoke_from_event_loop(open);
        }
        "quit" => {
            let _ = slint::invoke_from_event_loop(quit);
        }
        _ => {}
    }));
    TrayIconBuilder::new()
        .with_icon(Icon::from_resource(1, Some((16, 16))).expect("icon resource"))
        .with_tooltip("RDM")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .expect("tray icon")
}
