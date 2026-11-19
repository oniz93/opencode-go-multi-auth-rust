use std::sync::{Arc, Mutex};

use gpui_kit::component::Theme;
use gpui_kit::*;

use ocrouter_gui::client::{self, CONTROL_PORT};
use ocrouter_gui::state::{control_token, UiState};
use ocrouter_gui::view::Dashboard;

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
            client::spawn_poller(shared, base, control_token());
        });
    }

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let shared = Arc::clone(&shared);
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("ocrouter".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(920.), px(760.)),
                cx,
            ))),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, move |window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            let base = format!("http://127.0.0.1:{CONTROL_PORT}");
            cx.new(|cx| Dashboard::new(Arc::clone(&shared), base, window, cx))
        })
        .expect("failed to open the ocrouter window");
    });
}
