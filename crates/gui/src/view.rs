use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme, Disableable, Selectable, Sizable, Theme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::state::{
    clock, compact, control_token, remaining, AllowanceRow, EventRow, KeyRow, Routing, Step,
    UiState,
};

const PROVIDERS: [(&str, &str); 3] = [
    ("opencode-console", "Zen"),
    ("experiential", "Experiential"),
    ("opencode-go", "Go"),
];

fn provider_name(provider: &str) -> &str {
    PROVIDERS
        .iter()
        .find(|(id, _)| *id == provider)
        .map(|(_, name)| *name)
        .unwrap_or(provider)
}

pub struct Dashboard {
    state: Arc<Mutex<UiState>>,
    base: String,
    /// Result of the last control-API write, shown under the config card.
    notice: Arc<Mutex<Option<String>>>,
    /// Unsaved routing edits; `None` follows what the daemon reports.
    draft: Option<Routing>,
    provider: &'static str,
    label: Entity<InputState>,
    secret: Entity<InputState>,
    alias: Entity<InputState>,
    step_model: Entity<InputState>,
}

impl Dashboard {
    pub fn new(
        state: Arc<Mutex<UiState>>,
        base: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input =
            |placeholder: &str, masked: bool, window: &mut Window, cx: &mut Context<Self>| {
                let placeholder = placeholder.to_string();
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(placeholder)
                        .masked(masked)
                })
            };
        let label = input("Label, e.g. zen-1", false, window, cx);
        let secret = input("API key", true, window, cx);
        let alias = input(
            "Exposed model id, e.g. deepseek-v4.1-flash",
            false,
            window,
            cx,
        );
        let step_model = input(
            "Upstream model for the next step (blank = route id)",
            false,
            window,
            cx,
        );
        // Repaint on a slow tick; the polling thread owns the network.
        cx.spawn(async move |this, cx| loop {
            smol::Timer::after(Duration::from_millis(500)).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        })
        .detach();
        Dashboard {
            state,
            base,
            notice: Arc::new(Mutex::new(None)),
            draft: None,
            provider: PROVIDERS[0].0,
            label,
            secret,
            alias,
            step_model,
        }
    }

    fn snapshot(&self) -> UiState {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn set_notice(&self, text: impl Into<String>) {
        if let Ok(mut notice) = self.notice.lock() {
            *notice = Some(text.into());
        }
    }

    /// Fire a control-API write on a std thread; gpui's executor is not tokio.
    fn api(&self, method: reqwest::Method, path: String, body: Option<String>, done: &'static str) {
        let base = self.base.clone();
        let notice = Arc::clone(&self.notice);
        std::thread::spawn(move || {
            let result =
                crate::client::call(&base, control_token().as_deref(), method, &path, body);
            if let Ok(mut notice) = notice.lock() {
                *notice = Some(match result {
                    Ok(()) => done.to_string(),
                    Err(error) => format!("Failed: {error}"),
                });
            }
        });
    }

    fn edit(&mut self, f: impl FnOnce(&mut Routing)) {
        let mut routing = self.draft.take().unwrap_or_else(|| self.snapshot().routing);
        f(&mut routing);
        self.draft = Some(routing);
    }

    fn add_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let label = self.label.read(cx).value().trim().to_string();
        let secret = self.secret.read(cx).value().trim().to_string();
        if label.is_empty() || secret.is_empty() {
            self.set_notice("A label and a key are both required.");
            return;
        }
        let body =
            serde_json::json!({ "provider": self.provider, "label": label, "secret": secret });
        self.api(
            reqwest::Method::POST,
            "/api/keys".into(),
            Some(body.to_string()),
            "Key saved.",
        );
        self.label.update(cx, |s, cx| s.set_value("", window, cx));
        self.secret.update(cx, |s, cx| s.set_value("", window, cx));
    }

    fn add_route(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.alias.read(cx).value().trim().to_string();
        if model.is_empty() {
            self.set_notice("Type the model id to expose first.");
            return;
        }
        self.edit(|r| {
            if !r.routes.iter().any(|route| route.model == model) {
                r.routes.push(crate::state::RouteRow {
                    model,
                    steps: vec![],
                });
            }
        });
        self.alias.update(cx, |s, cx| s.set_value("", window, cx));
    }

    fn config_card(&self, state: &UiState, cx: &Context<Self>) -> Div {
        let t = cx.theme();
        let routing = self.draft.clone().unwrap_or_else(|| state.routing.clone());
        let section = |title: &str, hint: &str| {
            v_flex()
                .px_4()
                .py_3()
                .gap_2()
                .border_t_1()
                .border_color(t.border)
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(title.to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(t.muted_foreground)
                                .child(hint.to_string()),
                        ),
                )
        };

        // Add key
        let providers =
            h_flex()
                .gap_1()
                .children(PROVIDERS.iter().enumerate().map(|(pi, (id, name))| {
                    let id: &'static str = id;
                    Button::new(("provider", pi))
                        .small()
                        .label(*name)
                        .selected(self.provider == id)
                        .when(self.provider == id, |b| b.primary())
                        .when(self.provider != id, |b| b.outline())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.provider = id;
                            cx.notify();
                        }))
                }));
        let add_key = section(
            "Add key",
            "stored in keys.toml (0600) · re-adding a label replaces it",
        )
        .child(providers)
        .child(
            h_flex()
                .gap_2()
                .child(div().w(px(160.)).child(Input::new(&self.label)))
                .child(div().flex_1().child(Input::new(&self.secret).mask_toggle()))
                .child(
                    Button::new("add-key")
                        .primary()
                        .small()
                        .label("Add key")
                        .on_click(cx.listener(|this, _, window, cx| this.add_key(window, cx))),
                ),
        );

        // Routes
        let key_label = |id: &str| -> String {
            state
                .keys
                .iter()
                .find(|k| k.id == id)
                .map(|k| format!("{} · {}", k.label, provider_name(&k.provider)))
                .unwrap_or_else(|| format!("{id} (missing)"))
        };
        let mut routes = section(
            "Routes",
            "each step is used until it answers 429, then the next",
        );
        for (ri, route) in routing.routes.iter().enumerate() {
            let mut block = v_flex()
                .gap_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(t.border)
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .font_family(t.mono_font_family.clone())
                                .text_sm()
                                .child(route.model.clone()),
                        )
                        .child(
                            Button::new(("route-del", ri))
                                .ghost()
                                .xsmall()
                                .label("Remove route")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.edit(|r| {
                                        r.routes.remove(ri);
                                    });
                                    cx.notify();
                                })),
                        ),
                );
            let last = route.steps.len().saturating_sub(1);
            for (si, step) in route.steps.iter().enumerate() {
                let id = ri * 1000 + si;
                block = block.child(
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .child(
                            div()
                                .w(px(18.))
                                .text_color(t.muted_foreground)
                                .child(format!("{}.", si + 1)),
                        )
                        .child(div().w(px(200.)).truncate().child(key_label(&step.key)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(t.mono_font_family.clone())
                                .text_color(t.muted_foreground)
                                .child(format!("→ {}", step.model)),
                        )
                        .child(
                            Button::new(("step-up", id))
                                .ghost()
                                .xsmall()
                                .label("↑")
                                .disabled(si == 0)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.edit(|r| r.routes[ri].steps.swap(si, si - 1));
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(("step-down", id))
                                .ghost()
                                .xsmall()
                                .label("↓")
                                .disabled(si == last)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.edit(|r| r.routes[ri].steps.swap(si, si + 1));
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(("step-del", id))
                                .ghost()
                                .xsmall()
                                .label("✕")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.edit(|r| {
                                        r.routes[ri].steps.remove(si);
                                    });
                                    cx.notify();
                                })),
                        ),
                );
            }
            let add_step = h_flex()
                .flex_wrap()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child("Add step:"),
                )
                .children(state.keys.iter().enumerate().map(|(ki, key)| {
                    let key_id = key.id.clone();
                    Button::new(("step-add", ri * 1000 + ki))
                        .outline()
                        .xsmall()
                        .label(format!(
                            "+ {} · {}",
                            key.label,
                            provider_name(&key.provider)
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let typed = this.step_model.read(cx).value().trim().to_string();
                            let key = key_id.clone();
                            this.edit(|r| {
                                let route = &mut r.routes[ri];
                                let model = if typed.is_empty() {
                                    route.model.clone()
                                } else {
                                    typed
                                };
                                route.steps.push(Step { key, model });
                            });
                            cx.notify();
                        }))
                }));
            routes = routes.child(block.child(add_step));
        }
        routes = routes
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.alias)))
                    .child(
                        Button::new("add-route")
                            .outline()
                            .small()
                            .label("Add route")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_route(window, cx)),
                            ),
                    ),
            )
            .child(Input::new(&self.step_model))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .children(state.models.iter().enumerate().map(|(mi, model)| {
                        let id = model.id.clone();
                        Button::new(("model-pick", mi))
                            .ghost()
                            .xsmall()
                            .label(format!("{} · {}", model.id, model.lane))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let id = id.clone();
                                this.step_model
                                    .update(cx, |s, cx| s.set_value(id, window, cx));
                            }))
                    })),
            );

        // Exposure
        let all = routing.listable(&state.models);
        let expose = section(
            "Exposed to OpenCode",
            "listed by /v1/models · then press Sync models",
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap_3()
                .children(all.iter().enumerate().map(|(i, id)| {
                    let (id, all) = (id.clone(), all.clone());
                    Checkbox::new(("expose", i))
                        .label(id.clone())
                        .checked(routing.is_exposed(&id))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.edit(|r| r.toggle_expose(&id, &all));
                            cx.notify();
                        }))
                })),
        );

        let notice = self.notice.lock().ok().and_then(|n| n.clone());
        let footer = h_flex()
            .px_4()
            .py_3()
            .gap_2()
            .border_t_1()
            .border_color(t.border)
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(t.muted_foreground)
                    .child(notice.unwrap_or_default()),
            )
            .child(
                Button::new("routing-discard")
                    .ghost()
                    .small()
                    .label("Discard")
                    .disabled(self.draft.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.draft = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("routing-save")
                    .primary()
                    .small()
                    .label("Save routing")
                    .disabled(self.draft.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(routing) = this.draft.take() {
                            if routing.routes.iter().any(|r| r.steps.is_empty()) {
                                this.set_notice("Every route needs at least one step.");
                                this.draft = Some(routing);
                                return;
                            }
                            // Show the saved version until the next poll confirms it.
                            if let Ok(mut s) = this.state.lock() {
                                s.routing = routing.clone();
                            }
                            this.api(
                                reqwest::Method::PUT,
                                "/api/routing".into(),
                                Some(routing.to_json()),
                                "Routing saved.",
                            );
                        }
                        cx.notify();
                    })),
            );

        card(t, "Configuration", "keys · routes · exposed models")
            .child(add_key)
            .child(routes)
            .child(expose)
            .child(footer)
    }
}

fn run_sibling(args: &'static [&'static str]) {
    if let Some(bin) = crate::client::sibling_bin("ocrouter") {
        let _ = std::process::Command::new(bin).args(args).spawn();
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn dot(color: Hsla) -> Div {
    div().size_2().flex_none().rounded_full().bg(color)
}

fn card(t: &Theme, title: &str, hint: &str) -> Div {
    v_flex()
        .rounded_lg()
        .border_1()
        .border_color(t.border)
        .bg(t.background)
        .child(
            h_flex()
                .px_4()
                .py_3()
                .gap_2()
                .justify_between()
                .border_b_1()
                .border_color(t.border)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child(hint.to_string()),
                ),
        )
}

fn empty(t: &Theme, text: &str) -> Div {
    div()
        .px_4()
        .py_6()
        .text_sm()
        .text_center()
        .text_color(t.muted_foreground)
        .child(text.to_string())
}

fn stat(t: &Theme, label: &str, value: String, accent: Option<Hsla>) -> Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(t.border)
        .bg(t.background)
        .child(
            div()
                .text_xs()
                .text_color(t.muted_foreground)
                .child(label.to_string()),
        )
        .child(
            div()
                .text_2xl()
                .font_weight(FontWeight::SEMIBOLD)
                .truncate()
                .text_color(accent.unwrap_or(t.foreground))
                .child(value),
        )
}

fn header(t: &Theme, connected: bool) -> Div {
    let (color, text) = if connected {
        (t.success, "Connected")
    } else {
        (t.danger, "Disconnected")
    };
    let mut actions = h_flex().gap_2().child(
        Button::new("sync-models")
            .outline()
            .small()
            .label("Sync models")
            .on_click(|_, _, _| run_sibling(&["models", "sync", "--write"])),
    );
    if !connected {
        actions = actions.child(
            Button::new("start-daemon")
                .primary()
                .small()
                .label("Start daemon")
                .on_click(|_, _, _| run_sibling(&["serve"])),
        );
    }
    h_flex()
        .px_6()
        .py_4()
        .gap_4()
        .border_b_1()
        .border_color(t.border)
        .bg(t.background)
        .child(
            v_flex()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("ocrouter"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child("OpenCode Go + Experiential Labs key pool"),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .px_2p5()
                .py_1()
                .rounded_full()
                .border_1()
                .border_color(t.border)
                .text_xs()
                .child(dot(color))
                .child(text),
        )
        .child(div().flex_1())
        .child(actions)
}

fn key_row(
    t: &Theme,
    i: usize,
    key: &KeyRow,
    now: u64,
    remove: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let cooldown = key
        .cooldown_until_ms
        .and_then(|until| remaining(until, now));
    let (color, tag) = if !key.enabled || key.status == "disabled" {
        (t.muted_foreground, Tag::secondary().child("Disabled"))
    } else if let Some(left) = cooldown {
        (t.warning, Tag::warning().child(format!("Cooling · {left}")))
    } else if key.status == "exhausted" {
        (t.warning, Tag::warning().child("Exhausted"))
    } else if key.status == "active" {
        (t.success, Tag::success().child("Active"))
    } else {
        (
            t.muted_foreground,
            Tag::secondary().child(key.status.clone()),
        )
    };
    let mut row = h_flex()
        .px_4()
        .py_3()
        .gap_3()
        .when(i > 0, |row| row.border_t_1().border_color(t.border))
        .child(dot(color))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(key.label.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child(key.provider.clone()),
                ),
        )
        .child(
            div()
                .font_family(t.mono_font_family.clone())
                .text_xs()
                .text_color(t.muted_foreground)
                .child(key.preview.clone()),
        );
    if key.consecutive_errors > 0 {
        let s = if key.consecutive_errors == 1 { "" } else { "s" };
        row = row.child(
            div()
                .text_xs()
                .text_color(t.danger)
                .child(format!("{} error{s}", key.consecutive_errors)),
        );
    }
    row.child(tag.small()).child(
        Button::new(("key-del", i))
            .ghost()
            .xsmall()
            .label("Remove")
            .on_click(remove),
    )
}

fn allowance_row(t: &Theme, i: usize, row: &AllowanceRow) -> Div {
    let percent = if row.limit == 0 {
        0.0
    } else {
        (row.used as f64 / row.limit as f64 * 100.0).min(100.0) as f32
    };
    let color = match percent {
        p if p >= 90.0 => t.danger,
        p if p >= 70.0 => t.warning,
        _ => t.success,
    };
    v_flex()
        .px_4()
        .py_3()
        .gap_2()
        .when(i > 0, |r| r.border_t_1().border_color(t.border))
        .child(
            h_flex()
                .gap_3()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(row.org.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child(format!(
                            "{} / {} tokens · {percent:.0}%",
                            compact(row.used),
                            compact(row.limit)
                        )),
                ),
        )
        .child(
            Progress::new(("allowance", i))
                .value(percent)
                .color(color)
                .accessibility_label(format!("{} allowance used", row.org)),
        )
}

fn event_row(t: &Theme, event: &EventRow) -> Div {
    let tag = match event.level.as_str() {
        "error" => Tag::danger(),
        "warn" => Tag::warning(),
        _ => Tag::secondary(),
    };
    h_flex()
        .px_4()
        .py_1p5()
        .gap_3()
        .text_xs()
        .child(
            div()
                .flex_none()
                .font_family(t.mono_font_family.clone())
                .text_color(t.muted_foreground)
                .child(clock(event.at_ms)),
        )
        .child(tag.xsmall().child(event.level.to_uppercase()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(event.message.clone()),
        )
}

impl Render for Dashboard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.snapshot();
        let t = cx.theme();
        let now = now_ms();

        let active = state
            .keys
            .iter()
            .filter(|k| k.enabled && k.status == "active")
            .count();
        let cooling = state
            .keys
            .iter()
            .filter(|k| {
                k.cooldown_until_ms
                    .and_then(|u| remaining(u, now))
                    .is_some()
            })
            .count();
        let strategy = if state.strategy.is_empty() {
            "—".to_string()
        } else {
            state.strategy.replace('_', " ")
        };

        let mut content = v_flex().p_6().gap_5();

        if let Some(error) = &state.error {
            content =
                content.child(Alert::error("error", error.clone()).title("Daemon unreachable"));
        }

        content = content.child(
            h_flex()
                .gap_4()
                .child(stat(t, "Accounts", state.keys.len().to_string(), None))
                .child(stat(
                    t,
                    "Active",
                    active.to_string(),
                    (active > 0).then_some(t.success),
                ))
                .child(stat(
                    t,
                    "Cooling down",
                    cooling.to_string(),
                    (cooling > 0).then_some(t.warning),
                ))
                .child(stat(t, "Routing", strategy, None)),
        );

        let accounts = card(t, "Accounts", "keys are masked");
        content = content.child(if state.keys.is_empty() {
            accounts.child(empty(t, "No keys configured. Add one under Configuration."))
        } else {
            accounts.children(state.keys.iter().enumerate().map(|(i, k)| {
                let id = k.id.clone();
                key_row(
                    t,
                    i,
                    k,
                    now,
                    cx.listener(move |this, _, _, _| {
                        let path = format!("/api/keys/{id}");
                        this.api(reqwest::Method::DELETE, path, None, "Key removed.");
                    }),
                )
            }))
        });

        content = content.child(self.config_card(&state, cx));

        let allowance = card(t, "Experiential allowance", "per org · resets 00:00 UTC");
        content = content.child(if state.allowance.is_empty() {
            allowance.child(empty(t, "No allowance recorded yet."))
        } else {
            allowance.children(
                state
                    .allowance
                    .iter()
                    .enumerate()
                    .map(|(i, r)| allowance_row(t, i, r)),
            )
        });

        let events = card(t, "Recent events", "newest first · UTC");
        content = content.child(if state.events.is_empty() {
            events.child(empty(t, "No events yet."))
        } else {
            events.child(
                v_flex()
                    .py_2()
                    .children(state.events.iter().map(|e| event_row(t, e))),
            )
        });

        v_flex()
            .size_full()
            .bg(t.muted)
            .text_color(t.foreground)
            .child(header(t, state.connected))
            .child(
                div()
                    .id("content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(content),
            )
    }
}
