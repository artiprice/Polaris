# Web UI: plan and decisions

> 2026-10-29. Scope from the task list: the web version takes the welcome page of
> `dist/demo-design-preview-slop/webui/index.html` (an unoptimised static preview) and builds
> everything else after the Slint desktop GUI; page transitions and animations in both the web UI
> and the desktop GUI; a final UX pass over CLI, desktop and web.

## 1. Architecture

| Layer | Decision | Why |
|---|---|---|
| Server | `polaris-ui-web`: axum router on loopback (`polaris web`, default `127.0.0.1:8787`) | axum is already in the workspace; the session host, provider chain and kernel stay in-process, exactly as in the terminal and desktop. |
| Conversation | `polaris_ui_session::ConversationHost` + `OfflineSession` | One implementation for all three frontends: same provider chain, agent with reviewed file changes, model switching, interruption. |
| Transport | JSON for commands (`POST /api/…`), Server-Sent Events for the live stream (`GET /api/events`) | SSE is one-way, survives proxies, needs no extra client library; commands are plain fetches. |
| Frontend | Embedded static HTML/CSS/JS (no build step), served from the binary (`include_str!`) | Leptos (the original plan) needs `wasm32-unknown-unknown` and `cargo-leptos`, which are not installed on the team machines; the page must work offline from the binary. Leptos stays the upgrade path: the API is the contract. |
| Security | Loopback bind only; a per-run random token in the page URL and required on every API call (`Authorization: Bearer`); `Host` header checked against the bound address; strict CSP (no inline script, `connect-src 'self'`) | A local web UI is reachable by any page in the browser: DNS rebinding and CSRF are the real threats. |

## 2. Pages (after the Slint GUI)

- **Welcome** — the slop preview's welcome page, optimised: one stylesheet, CSS-only aurora
  (gradients + 3 star layers, no images), `prefers-reduced-motion` respected, composer pill with
  the model menu fed by the provider's effort table.
- **Session** — sidebar (sessions grouped by workspace, search), conversation (user bubbles,
  assistant Markdown with code blocks, system notes, errors), docked composer with Stop while a
  reply streams, status line (`model · provider` or `Working 12s · Esc to stop`).
- **Models / Usage / Settings** — popover panels as in the desktop: models per effort (click to
  switch), this session's tokens + the cross-session activity calendar, about + appearance.

## 3. Motion

- Page transitions: the View Transitions API (`document.startViewTransition`) cross-fades welcome →
  session with the composer as a shared element; a CSS fallback fades for browsers without it.
- Panels and menus: 160 ms opacity + 6 px rise; streaming text appears without layout jumps.
- Desktop (Slint): welcome → session cross-fade (opacity animation on both screens), panels
  already fade; new messages fade in.
- Everything honours reduced motion (`prefers-reduced-motion`, desktop `--static` /
  `POLARIS_REDUCED_MOTION`).

## 4. API

| Method | Path | Body / result |
|---|---|---|
| GET | `/` | the app (token in the query string, then moved to `sessionStorage`) |
| GET | `/api/state` | provider, models, sessions, open session |
| POST | `/api/open` | `{resume?: id}` |
| POST | `/api/submit` | `{text, plan}` |
| POST | `/api/interrupt` | — |
| POST | `/api/effort` | `{effort}` |
| GET | `/api/stats` | activity lines |
| GET | `/api/events` | SSE: `message`, `reply-started`, `reply-text`, `reply-step`, `reply-finished`, `reply-failed`, `provider`, `usage`, `sessions`, `notice` |

## 5. Done when

- `polaris web` opens a working UI: send a prompt, watch the streamed reply, stop it, switch
  model, open another session, see usage.
- Router tests cover auth (missing/wrong token, foreign Host), state, submit → reply stream
  against a loopback fake provider.
- The UX pass checklist (§6) is applied to all three frontends.

## 6. UX pass checklist

Spacing on a 4 px grid; one type scale per surface; line height 1.5 for prose, 1.35 for UI text;
every popover dismissible by Esc and outside click; focus rings visible; disabled controls say
why; empty states explain the next step; errors name the cause and the fix; no truncated text
without a tooltip; status colours: green connected, amber connecting/working, red failed.

## 7. Leptos frontend (`apps/polaris-web`, 2026-10-29)

The upgrade path from §1 is built. `polaris-web` serves the same API, event stream, stylesheet and
fonts as `polaris-ui-web` (`WebServer::bind_with_pages`) and replaces the embedded JavaScript page
with Leptos pages:

- **SSR + hydration.** The server streams the HTML (out-of-order) with a per-response CSP nonce
  (`script-src 'self' 'nonce-…' 'wasm-unsafe-eval'`, `frame-ancestors 'none'`); the browser hydrates
  the same components from the wasm bundle under `/pkg`.
- **Behaviour in Rust.** `state::UiState` reduces server events and clicks into network effects
  (`Open`, `Submit`, `Interrupt`, `Review`, `Effort`, `RefreshSessions`); the browser client performs
  them with the token from `#token=` (kept in `sessionStorage`, removed from the address bar).
- **PWA.** `/manifest.webmanifest`, `/icon.svg` and `/sw.js` — the only hand-written JavaScript:
  network-first pages with an offline shell, cache-first bundle/styles/fonts, never the API.
- **Build and run.**
  ```
  rustup target add wasm32-unknown-unknown --toolchain 1.99.0
  cargo +1.99.0 install cargo-leptos --locked
  cd apps/polaris-web && cargo leptos build
  target/debug/polaris-web --site-root target/site --env-file <keys>.env
  ```
- **Not yet:** per-route lazy loading (one route today), WebSocket terminal channels, attaching to
  `polarisd` over `polaris-rpc` instead of an in-process session owner.
