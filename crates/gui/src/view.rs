use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::label::Label;
use gpui_kit::base::StyledExt;
use gpui_kit::*;

use crate::state::{AllowanceRow, KeyRow, UiState};

pub struct Dashboard {
    state: Arc<Mutex<UiState>>,
}

impl Dashboard {
    pub fn new(state: Arc<Mutex<UiState>>, cx: &mut Context<Self>) -> Self {
        // Repaint on a slow tick; the polling thread owns the network.
        cx.spawn(async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(500)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        Dashboard { state }
    }

    fn snapshot(&self) -> UiState {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn section_title(text: &str) -> impl IntoElement {
    div().child(Label::new(text.to_string()))
}

fn key_row(key: &KeyRow) -> impl IntoElement {
    let status = match &key.cooldown_until_ms {
        Some(_) => "cooling down".to_string(),
        None => key.status.clone(),
    };
    div()
        .h_flex()
        .gap_3()
        .items_center()
        .child(Label::new(key.label.clone()))
        .child(Label::new(key.provider.clone()))
        .child(Label::new(key.preview.clone()))
        .child(Label::new(status))
}

fn allowance_row(row: &AllowanceRow) -> impl IntoElement {
    let percent = if row.limit == 0 {
        0.0
    } else {
        (row.used as f64 / row.limit as f64 * 100.0).min(100.0)
    };
    div()
        .h_flex()
        .gap_3()
        .items_center()
        .child(Label::new(row.org.clone()))
        .child(Label::new(format!(
            "{:.0}%  ({} / {} tokens)",
            percent, row.used, row.limit
        )))
}

impl Render for Dashboard {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let state = self.snapshot();

        let mut root = div().v_flex().size_full().p_6().gap_4();

        root = root.child(
            div()
                .h_flex()
                .gap_3()
                .items_center()
                .child(Label::new("ocrouter".to_string()))
                .child(Label::new(if state.connected {
                    "connected".to_string()
                } else {
                    "disconnected".to_string()
                }))
                .child(Label::new(format!("strategy: {}", state.strategy)))
                .child(
                    Button::new("start-daemon")
                        .primary()
                        .label("Start daemon")
                        .on_click(|_, _, _| {
                            if let Some(bin) = crate::client::sibling_bin("ocrouter") {
                                let _ = std::process::Command::new(bin)
                                    .arg("serve")
                                    .spawn();
                            }
                        }),
                )
                .child(
                    Button::new("sync-models")
                        .label("Sync models")
                        .on_click(|_, _, _| {
                            if let Some(bin) = crate::client::sibling_bin("ocrouter") {
                                let _ = std::process::Command::new(bin)
                                    .args(["models", "sync", "--write"])
                                    .spawn();
                            }
                        }),
                ),
        );

        root = root.child(section_title("Accounts"));
        if state.keys.is_empty() {
            root = root.child(Label::new("no keys configured".to_string()));
        } else {
            root = root.children(state.keys.iter().map(key_row));
        }

        root = root.child(section_title("Experiential allowance (per org, resets 00:00 UTC)"));
        if state.allowance.is_empty() {
            root = root.child(Label::new("no allowance recorded yet".to_string()));
        } else {
            root = root.children(state.allowance.iter().map(allowance_row));
        }

        root = root.child(section_title("Recent events"));
        if state.events.is_empty() {
            root = root.child(Label::new("no events yet".to_string()));
        } else {
            root = root.children(state.events.iter().map(|e| Label::new(e.clone())));
        }

        if let Some(error) = &state.error {
            root = root.child(Label::new(format!("error: {error}")));
        }

        root
    }
}
