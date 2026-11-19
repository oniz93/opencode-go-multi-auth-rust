use std::sync::{Arc, Mutex};

use gpui_kit::*;

use ocrouter_gui::client::{self, CONTROL_PORT};
use ocrouter_gui::state::{control_token, UiState};
use ocrouter_gui::view::Dashboard;

gpui_kit::actions!(ocrouter, [Quit]);

fn main() {
    let shared = Arc::new(Mutex::new(UiState::default()));
    let base = format!("http://127.0.0.1:{CONTROL_PORT}");

    {
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            // Bring the daemon up before the first poll if it is not running.
            if let Some(bin) = client::sibling_bin("ocrouter") {
                let _ = client::ensure_daemon(&bin, CONTROL_PORT);
            }
            let token = control_token();
            client::spawn_poller(shared, base, token);
        });
    }

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        // GPUI ships no default quit: Cmd+Q needs an action, a binding, and a
        // menu item (macOS routes the shortcut through the menu bar).
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus([Menu::new("Ocrouter").items([MenuItem::action("Quit Ocrouter", Quit)])]);
        // One-window app: closing it quits (the daemon keeps running).
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let shared = Arc::clone(&shared);
        gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
            cx.new(|cx| Dashboard::new(Arc::clone(&shared), cx))
        })
        .expect("failed to open the ocrouter window");
    });
}
