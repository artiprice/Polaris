//! # polaris-ui
//!
//! Facade over the user-interface crates. Frontends depend on the children directly; this crate
//! names them in one place and records the rules they share.
//!
//! | Module | Crate | What it is |
//! |---|---|---|
//! | [`model`] | `polaris-ui-model` | Frontend-neutral state: composer, conversation view, completion, design tokens |
//! | [`session`] | `polaris-ui-session` | One local session and the shared conversation host (provider chain, agent, reviews) |
//! | [`cli`] | `polaris-ui-cli` | Terminal rendering, input decoding and the question overlay |
//! | [`gui`] | `polaris-ui-gui` | Slint desktop window: welcome aurora, session view, panels |
//! | [`web`] | `polaris-ui-web` | Loopback web UI: embedded page, JSON API, Server-Sent Events |
//! | [`ascii_aurora`] | `polaris-ui-ascii-aurora` | The ASCII aurora renderer behind the welcome screens |
//!
//! ## Design tokens
//!
//! Colours, radius, motion speed and fonts come from [`model::tokens`]; the desktop build
//! generates its Slint `Tokens` global from them and the web stylesheet mirrors the same values,
//! so a role means the same colour everywhere.
//!
//! ## UX rules every frontend follows
//!
//! - **One conversation implementation.** Replies, model switching, plan mode, interruption and
//!   change reviews all go through [`session::ConversationHost`]; a frontend only translates its
//!   events.
//! - **Nothing is written without the person.** Outside plan mode the agent may propose file
//!   changes; each one shows a minimal diff and waits for an explicit apply or decline (with an
//!   optional note for the model). Plan mode only reads.
//! - **Drafts survive failures.** A rejected or unsent message stays in the composer.
//! - **Work is visible.** A running reply shows elapsed time and how to stop it (Esc / Stop); a
//!   finished one ends with a one-line summary of requests, tool rounds and changed files.
//! - **Motion is optional.** Reduced-motion settings turn off cosmetic animation everywhere.
//! - **Local by default.** The web UI listens on loopback only, takes its token from the URL
//!   fragment and checks the `Host` header; provider keys come from `--env-file` or
//!   `~/.polaris/.env`, never from a repository `.env`.

pub use polaris_ui_ascii_aurora as ascii_aurora;
pub use polaris_ui_cli as cli;
pub use polaris_ui_gui as gui;
pub use polaris_ui_model as model;
pub use polaris_ui_session as session;
pub use polaris_ui_web as web;
