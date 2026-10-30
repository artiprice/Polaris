//! The page: welcome rail or session sidebar, conversation with review cards, composer and
//! panels. It reuses the class names and stylesheet of the embedded web UI (`/app.css`), so
//! both frontends look the same; behaviour lives in [`UiState`] and the browser client.

use leptos::prelude::*;
use leptos_meta::{Title, provide_meta_context};
use leptos_router::{
    components::{Route, Router, Routes},
    path,
};

use crate::markdown::{Block, Inline, blocks, diff_class};
use crate::state::{Connection, Effect as Network, EntryKind, ReviewState, UiState};

/// Perform network effects (in the browser; nothing on the server).
fn run(ui: RwSignal<UiState>, effects: Vec<Network>) {
    #[cfg(feature = "hydrate")]
    crate::client::run(ui, effects);
    #[cfg(not(feature = "hydrate"))]
    let _ = (ui, effects);
}

const BRAND: &str = "M15 15h34v34H15z";

/// The whole app; the server renders it inside [`crate::server::shell`].
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    let ui = RwSignal::new(UiState::default());
    provide_context(ui);
    #[cfg(feature = "hydrate")]
    Effect::new(move |started: Option<()>| {
        if started.is_none() {
            crate::client::start(ui);
        }
    });
    view! {
        <Title text="Polaris"/>
        <Router>
            <Routes fallback=|| view! { <p class="muted">"Page not found."</p> }>
                <Route path=path!("/") view=Workspace/>
            </Routes>
        </Router>
    }
}

#[component]
fn Workspace() -> impl IntoView {
    let ui = expect_context::<RwSignal<UiState>>();
    let panel = RwSignal::new(None::<&'static str>);
    let seconds = RwSignal::new(0_u64);
    #[cfg(feature = "hydrate")]
    {
        set_interval(
            move || {
                if ui.with_untracked(|state| state.working) {
                    seconds.update(|value| *value = value.saturating_add(1));
                }
            },
            std::time::Duration::from_secs(1),
        );
        Effect::new(move |_| {
            if !ui.with(|state| state.working) {
                seconds.set(0);
            }
        });
        let _escape = window_event_listener(leptos::ev::keydown, move |event| {
            if event.key() == "Escape" {
                if panel.get_untracked().is_some() {
                    panel.set(None);
                } else if ui.with_untracked(|state| state.working) {
                    let effects = ui.try_update(UiState::submit).unwrap_or_default();
                    run(ui, effects);
                }
            }
        });
    }
    let view_name = move || {
        if ui.with(|state| state.session.is_some()) {
            "session"
        } else {
            "welcome"
        }
    };
    let working = move || ui.with(|state| state.working).then_some("");
    let toggle = move |name: &'static str| {
        panel.update(|open| {
            *open = if *open == Some(name) {
                None
            } else {
                Some(name)
            };
        });
    };
    view! {
        <a class="skip-link" href="#composer-text">"Skip to the message box"</a>
        <div class="app" data-view=view_name data-working=working>
            <nav class="rail" aria-label="Main">
                <span class="brand" aria-hidden="true">
                    <svg viewBox="0 0 64 64" fill="none">
                        <path fill="#1C1C1C" d=BRAND/>
                        <path d="M21 32h8m6 0h8" stroke="#76B900"/>
                        <path d="m32 27 1.7 3.3L37 32l-3.3 1.7L32 37l-1.7-3.3L27 32l3.3-1.7L32 27Z" fill="#24D046"/>
                    </svg>
                </span>
                <button class="rail-button" type="button" aria-label="Models" data-tip="Models" on:click=move |_| toggle("models")>
                    <svg class="icon" viewBox="0 0 24 24"><rect x="4" y="4" width="6" height="6" rx="1"/><rect x="14" y="4" width="6" height="6" rx="1"/><rect x="4" y="14" width="6" height="6" rx="1"/><rect x="14" y="14" width="6" height="6" rx="1"/></svg>
                </button>
                <button class="rail-button" type="button" aria-label="Usage" data-tip="Usage" on:click=move |_| toggle("usage")>
                    <svg class="icon" viewBox="0 0 24 24"><path d="M4 4v16h16M8 15l4-5 4 2 4-7"/></svg>
                </button>
            </nav>
            <Sidebar panel=panel/>
            <main class="main">
                <header class="topbar">
                    <h1 class="session-title">
                        {move || ui.with(|state| match &state.session {
                            Some(session) if !session.title.is_empty() => session.title.clone(),
                            Some(_) => "Untitled session".to_owned(),
                            None => "New session".to_owned(),
                        })}
                    </h1>
                </header>
                <section class="welcome" aria-labelledby="welcome-title">
                    <div class="atmosphere" aria-hidden="true">
                        <span class="light mesh"></span>
                        <span class="light nebula"></span>
                        <span class="light aurora"></span>
                        <span class="light aurora two"></span>
                        <span class="grain"></span>
                    </div>
                    <h2 id="welcome-title">"What’s on your mind for Polaris?"</h2>
                </section>
                <Conversation/>
                <Composer seconds=seconds panel=panel/>
            </main>
            <Panels panel=panel/>
        </div>
    }
    .into_any()
}

#[component]
fn Sidebar(panel: RwSignal<Option<&'static str>>) -> impl IntoView {
    let ui = expect_context::<RwSignal<UiState>>();
    let filter = RwSignal::new(String::new());
    let rows = move || {
        let needle = filter.get().to_lowercase();
        ui.with(|state| {
            state
                .sessions
                .iter()
                .filter(|row| {
                    needle.is_empty()
                        || row.title.to_lowercase().contains(&needle)
                        || row.workspace.to_lowercase().contains(&needle)
                })
                .cloned()
                .collect::<Vec<_>>()
        })
    };
    view! {
        <aside class="sidebar" aria-label="Sessions">
            <div class="sidebar-head">
                <span class="wordmark">"Polaris"</span>
            </div>
            <button class="side-action" type="button" on:click=move |_| {
                let effects = ui.try_update(|state| {
                    state.session = None;
                    state.entries.clear();
                    Vec::new()
                }).unwrap_or_default();
                run(ui, effects);
            }>
                <svg class="icon" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>
                "New session"
            </button>
            <label class="side-search">
                <svg class="icon" viewBox="0 0 24 24"><circle cx="11" cy="11" r="7"/><path d="m21 21-4.35-4.35"/></svg>
                <input type="search" placeholder="Filter sessions" aria-label="Filter sessions" autocomplete="off"
                    prop:value=move || filter.get()
                    on:input=move |event| filter.set(event_target_value(&event))/>
            </label>
            <div class="session-list" role="list">
                <For each=rows key=|row| row.id.clone() let:row>
                    {
                        let id = row.id.clone();
                        let current = {
                            let id = id.clone();
                            move || ui.with(|state| state.session.as_ref().is_some_and(|open| open.id == id)).to_string()
                        };
                        let title = if row.title.is_empty() { "Untitled session".to_owned() } else { row.title.clone() };
                        view! {
                            <button class="session-row" type="button" role="listitem" aria-current=current
                                on:click=move |_| {
                                    let effects = ui.try_update(|state| state.open_listed(&id)).unwrap_or_default();
                                    run(ui, effects);
                                }>
                                <span class="name">{title}</span>
                            </button>
                        }
                    }
                </For>
            </div>
            <div class="sidebar-foot">
                <div class="foot-text">
                    <strong>{move || ui.with(|state| state.session.as_ref().map(|open| folder(&open.workspace)).unwrap_or_default())}</strong>
                    <span class="muted">{move || ui.with(|state| match &state.connection {
                        Connection::Ready { model, .. } => model.clone(),
                        Connection::Connecting => "Connecting…".to_owned(),
                        Connection::Offline(_) if state.attached() => "polarisd".to_owned(),
                        Connection::Offline(_) => "Offline".to_owned(),
                    })}</span>
                </div>
                <button class="foot-button" type="button" aria-label="Models" data-tip="Models" on:click=move |_| panel.set(Some("models"))>
                    <svg class="icon" viewBox="0 0 24 24"><rect x="4" y="4" width="6" height="6" rx="1"/><rect x="14" y="4" width="6" height="6" rx="1"/><rect x="4" y="14" width="6" height="6" rx="1"/><rect x="14" y="14" width="6" height="6" rx="1"/></svg>
                </button>
            </div>
        </aside>
    }
    .into_any()
}

fn folder(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_owned()
}

#[component]
fn Conversation() -> impl IntoView {
    let ui = expect_context::<RwSignal<UiState>>();
    let section = NodeRef::<leptos::html::Section>::new();
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        // Follow new content.
        let _ = ui.with(|state| {
            state.entries.len()
                + state.entries.last().map_or(0, |entry| match &entry.kind {
                    EntryKind::Assistant { text, .. } => text.len(),
                    _ => 0,
                })
        });
        if let Some(element) = section.get() {
            element.set_scroll_top(element.scroll_height());
        }
    });
    view! {
        <section class="conversation" aria-live="polite" aria-label="Conversation" node_ref=section>
            <For each=move || ui.with(|state| state.entries.clone()) key=|entry| (entry.id, entry.kind.clone()) let:entry>
                {render_entry(ui, entry.kind)}
            </For>
        </section>
    }
    .into_any()
}

const GLYPH_STAR: &str = "m32 27 1.7 3.3L37 32l-3.3 1.7L32 37l-1.7-3.3L27 32l3.3-1.7L32 27Z";

fn render_entry(ui: RwSignal<UiState>, kind: EntryKind) -> AnyView {
    match kind {
        EntryKind::User(text) | EntryKind::Directive(text) => view! {
            <div class="entry user"><div class="bubble">{text}</div></div>
        }
        .into_any(),
        EntryKind::Assistant { text, streaming } => view! {
            <div class="entry assistant">
                <span class="glyph" aria-hidden="true">
                    <svg viewBox="0 0 64 64" fill="none">
                        <path fill="#1C1C1C" d=BRAND/>
                        <path d="M21 32h8m6 0h8" stroke="#76B900"/>
                        <path d=GLYPH_STAR fill="#24D046"/>
                    </svg>
                </span>
                <div class="prose">
                    {if text.is_empty() && streaming {
                        view! { <span class="typing" aria-label="Working"><span></span><span></span><span></span></span> }.into_any()
                    } else {
                        markdown_view(&text).into_any()
                    }}
                </div>
            </div>
        }
        .into_any(),
        EntryKind::System(text) => view! { <div class="entry system">{text}</div> }.into_any(),
        EntryKind::Warning(text) => view! { <div class="entry warning">{text}</div> }.into_any(),
        EntryKind::Error(text) => view! { <div class="entry error">{text}</div> }.into_any(),
        EntryKind::Change(text) => view! { <div class="entry change">{text}</div> }.into_any(),
        EntryKind::Review {
            request_id,
            prompt,
            state,
        } => review_card(ui, request_id, &prompt, state).into_any(),
    }
}

fn review_card(
    ui: RwSignal<UiState>,
    request_id: String,
    prompt: &str,
    state: ReviewState,
) -> impl IntoView + use<> {
    let note = RwSignal::new(String::new());
    let decide = move |apply: bool, request_id: &str| {
        let text = note.get_untracked();
        let effects = ui
            .try_update(|current| current.decide(request_id, apply, &text))
            .unwrap_or_default();
        run(ui, effects);
    };
    let closed = match state {
        ReviewState::Waiting | ReviewState::Deciding { .. } => None,
        ReviewState::Applied => Some(("applied", "Applied")),
        ReviewState::Declined => Some(("declined", "Not applied")),
        ReviewState::Withdrawn => Some(("withdrawn", "Withdrawn")),
    };
    let deciding = matches!(state, ReviewState::Deciding { .. });
    let apply_id = request_id.clone();
    let decline_id = request_id.clone();
    let enter_id = request_id;
    view! {
        <section class="entry review" aria-label="Change waiting for your decision" data-closed=closed.map(|(key, _)| key)>
            <div class="review-head">"Review change"</div>
            <div class="prose">{markdown_view(prompt)}</div>
            {match closed {
                Some((_, label)) => view! { <div class="review-outcome">{label}</div> }.into_any(),
                None => view! {
                    <div class="review-actions">
                        <input type="text" class="review-note" maxlength="2000" placeholder="Note for Polaris (optional)"
                            aria-label="Note for Polaris" disabled=deciding
                            prop:value=move || note.get()
                            on:input=move |event| note.set(event_target_value(&event))
                            on:keydown=move |event| {
                                if event.key() == "Enter" && !note.get_untracked().trim().is_empty() {
                                    event.prevent_default();
                                    decide(false, &enter_id);
                                }
                            }/>
                        <button type="button" class="review-decline" disabled=deciding on:click=move |_| decide(false, &decline_id)>"Don't apply"</button>
                        <button type="button" class="review-apply" disabled=deciding on:click=move |_| decide(true, &apply_id)>"Apply"</button>
                    </div>
                }.into_any(),
            }}
        </section>
    }
}

fn inline_view(runs: Vec<Inline>) -> impl IntoView {
    runs.into_iter()
        .map(|run| match run {
            Inline::Text(text) => view! { <span>{text}</span> }.into_any(),
            Inline::Code(code) => view! { <code>{code}</code> }.into_any(),
        })
        .collect_view()
}

fn markdown_view(text: &str) -> impl IntoView + use<> {
    blocks(text)
        .into_iter()
        .map(|block| match block {
            Block::Heading(runs) => view! { <h3>{inline_view(runs)}</h3> }.into_any(),
            Block::Paragraph(lines) => {
                let count = lines.len();
                view! {
                    <p>
                        {lines.into_iter().enumerate().map(|(index, line)| view! {
                            {inline_view(line)}
                            {(index + 1 < count).then(|| view! { <br/> })}
                        }).collect_view()}
                    </p>
                }
                .into_any()
            }
            Block::Item(runs) => view! { <ul><li>{inline_view(runs)}</li></ul> }.into_any(),
            Block::Code { language, lines } => {
                let label = if language.is_empty() {
                    "text".to_owned()
                } else {
                    language.clone()
                };
                view! {
                    <div class="code">
                        <div class="code-head"><span>{label}</span></div>
                        <pre>
                            {lines.into_iter().map(|line| {
                                let class = diff_class(&language, &line);
                                view! { <span class=class>{format!("{line}\n")}</span> }
                            }).collect_view()}
                        </pre>
                    </div>
                }
                .into_any()
            }
        })
        .collect_view()
}

#[component]
fn Composer(seconds: RwSignal<u64>, panel: RwSignal<Option<&'static str>>) -> impl IntoView {
    let ui = expect_context::<RwSignal<UiState>>();
    let submit = move || {
        let effects = ui.try_update(UiState::submit).unwrap_or_default();
        run(ui, effects);
    };
    let send_disabled = move || {
        ui.with(|state| {
            !state.working && (state.composer.trim().is_empty() || state.pending.is_some())
        })
    };
    let status = move || ui.with(|state| state.status(seconds.get()));
    view! {
        <form class="composer" autocomplete="off" on:submit=move |event| {
            event.prevent_default();
            submit();
        }>
            <div class="composer-box">
                <label class="sr-only" for="composer-text">"Message for Polaris"</label>
                <textarea id="composer-text" rows="1" placeholder="Ask Polaris anything…" spellcheck="false"
                    prop:value=move || ui.with(|state| state.composer.clone())
                    on:input=move |event| {
                        let text = event_target_value(&event);
                        ui.update(|state| state.composer = text);
                    }
                    on:keydown=move |event| {
                        if event.key() == "Enter" && !event.shift_key() {
                            event.prevent_default();
                            submit();
                        }
                    }></textarea>
                <div class="composer-row">
                    <button class="mode-chip" type="button"
                        aria-pressed=move || ui.with(|state| state.plan).to_string()
                        title="Auto: Polaris may change files and asks before each change. Plan: read-only, answers with a plan."
                        on:click=move |_| ui.update(|state| state.plan = !state.plan)>
                        <span class="dot"></span>
                        <span>{move || if ui.with(|state| state.plan) { "Plan" } else { "Auto" }}</span>
                    </button>
                    <span class="spacer"></span>
                    <button class="model-pill" type="button" on:click=move |_| panel.set(Some("models"))>
                        <span>{move || ui.with(|state| match &state.connection {
                            Connection::Ready { model, .. } => short_model(model),
                            Connection::Connecting => "Connecting…".to_owned(),
                            Connection::Offline(_) => "No model".to_owned(),
                        })}</span>
                        <svg class="icon" viewBox="0 0 24 24"><path d="m8 10 4 4 4-4"/></svg>
                    </button>
                    <button class="send" type="submit"
                        aria-label=move || if ui.with(|state| state.working) { "Stop the reply" } else { "Send message" }
                        disabled=send_disabled>
                        <svg class="icon send-icon" viewBox="0 0 24 24"><path d="M12 19V5m-6 6 6-6 6 6"/></svg>
                        <svg class="icon stop-icon" viewBox="0 0 24 24"><rect x="7" y="7" width="10" height="10" rx="1.5"/></svg>
                    </button>
                </div>
            </div>
            <Show when=move || ui.with(|state| state.notice.is_some())>
                <p class="notice" role="status">{move || ui.with(|state| state.notice.clone().unwrap_or_default())}</p>
            </Show>
            <p class="status-line">
                <span class="status-dot" data-state=move || status().0></span>
                <span>{move || status().1}</span>
            </p>
        </form>
    }
    .into_any()
}

/// `vendor/model-name:free` → `model-name`.
fn short_model(model: &str) -> String {
    let name = model.rsplit('/').next().unwrap_or(model);
    name.split(':').next().unwrap_or(name).to_owned()
}

#[component]
fn Panels(panel: RwSignal<Option<&'static str>>) -> impl IntoView {
    let ui = expect_context::<RwSignal<UiState>>();
    view! {
        <Show when=move || panel.get() == Some("models")>
            <section class="panel" role="dialog" aria-labelledby="models-title">
                <h2 id="models-title">"Models"</h2>
                <p class="muted">{move || ui.with(|state| match &state.connection {
                    Connection::Ready { provider, model } => format!("{model} · {provider}"),
                    Connection::Connecting => "Connecting…".to_owned(),
                    Connection::Offline(reason) => format!("No model provider: {reason}"),
                })}</p>
                <div class="model-rows" role="list">
                    <For each=move || ui.with(|state| state.models.clone()) key=|row| row.clone() let:row>
                        {
                            let (effort, model) = row;
                            let current = {
                                let effort = effort.clone();
                                move || ui.with(|state| state.effort == effort).to_string()
                            };
                            let choose = effort.clone();
                            view! {
                                <button type="button" class="model-row" role="listitem" aria-current=current
                                    on:click=move |_| {
                                        let effects = ui.try_update(|state| state.choose_effort(&choose)).unwrap_or_default();
                                        run(ui, effects);
                                    }>
                                    <span class="effort">{effort}</span>
                                    <span class="name">{model}</span>
                                </button>
                            }
                        }
                    </For>
                </div>
            </section>
        </Show>
        <Show when=move || panel.get() == Some("usage")>
            <section class="panel" role="dialog" aria-labelledby="usage-title">
                <h2 id="usage-title">"Usage"</h2>
                <pre class="usage-lines">{move || ui.with(|state| {
                    let (requests, prompt, completion) = state.usage;
                    format!("This session\nRequests            {requests}\nPrompt tokens       {prompt}\nCompletion tokens   {completion}")
                })}</pre>
            </section>
        </Show>
    }
    .into_any()
}
