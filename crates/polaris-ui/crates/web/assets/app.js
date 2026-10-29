// Polaris web UI: one page, a JSON command API and a Server-Sent Events stream.
"use strict";

const $ = (selector) => document.querySelector(selector);
const app = $(".app");
const conversation = $("#conversation");
const textarea = $("#composer-text");
const sendButton = $("#send");
const notice = $("#notice");

const state = {
  token: "",
  openId: null,
  provider: null,
  models: [],
  effort: "",
  working: false,
  plan: false,
  pending: null,
  reply: null,
  usage: { requests: 0, prompt_tokens: 0, completion_tokens: 0 },
  sessions: [],
  startedAt: 0,
};

// ---- access token: from the URL fragment once, then from this tab's storage ----------------
function readToken() {
  const match = /token=([0-9a-f]+)/.exec(location.hash);
  if (match) {
    try { sessionStorage.setItem("polaris-token", match[1]); } catch (_) { /* storage blocked */ }
    history.replaceState(null, "", location.pathname);
    return match[1];
  }
  try { return sessionStorage.getItem("polaris-token") || ""; } catch (_) { return ""; }
}

async function api(path, body) {
  const options = { method: body === undefined ? "GET" : "POST", headers: { Authorization: `Bearer ${state.token}` } };
  if (body !== undefined) {
    options.headers["Content-Type"] = "application/json";
    options.body = JSON.stringify(body);
  }
  const response = await fetch(path, options);
  if (response.status === 401) {
    showNotice("This page has no valid access token. Open the link that `polaris web` printed.");
  }
  return response;
}

// ---- small helpers -------------------------------------------------------------------------
function showNotice(text, timeout) {
  notice.textContent = text;
  notice.hidden = !text;
  if (timeout) setTimeout(() => { if (notice.textContent === text) showNotice(""); }, timeout);
}

function escapeHtml(text) {
  return text.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

function relativeTime(ms) {
  const minutes = Math.floor((Date.now() - ms) / 60000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

function shortModel(model) {
  const last = model.split("/").pop() || model;
  return last.split(":")[0];
}

function folder(path) {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || path;
}

function reducedMotion() {
  return app.hasAttribute("data-reduce-motion") || matchMedia("(prefers-reduced-motion: reduce)").matches;
}

function setView(view) {
  if (app.dataset.view === view) return;
  const apply = () => { app.dataset.view = view; };
  if (document.startViewTransition && !reducedMotion()) document.startViewTransition(apply);
  else apply();
}

// ---- Markdown (escaped first; only a safe subset becomes markup) ----------------------------
function inline(text) {
  let html = escapeHtml(text);
  html = html.replace(/`([^`]+)`/g, "<code>$1</code>");
  html = html.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  html = html.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
  return html;
}

function renderMarkdown(source, into) {
  into.textContent = "";
  const lines = source.split("\n");
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const fence = /^\s*```\s*([\w+-]*)/.exec(line);
    if (fence) {
      const language = fence[1].toLowerCase();
      const body = [];
      i += 1;
      while (i < lines.length && !/^\s*```/.test(lines[i])) { body.push(lines[i]); i += 1; }
      i += 1;
      into.appendChild(codeBlock(language, body));
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      const element = document.createElement(heading[1].length <= 2 ? "h3" : "h4");
      element.innerHTML = inline(heading[2]);
      into.appendChild(element);
      i += 1;
      continue;
    }
    if (/^\s*([-*+]|\d+[.)])\s+/.test(line)) {
      const ordered = /^\s*\d+[.)]\s+/.test(line);
      const list = document.createElement(ordered ? "ol" : "ul");
      while (i < lines.length && /^\s*([-*+]|\d+[.)])\s+/.test(lines[i])) {
        const item = document.createElement("li");
        item.innerHTML = inline(lines[i].replace(/^\s*([-*+]|\d+[.)])\s+/, ""));
        list.appendChild(item);
        i += 1;
      }
      into.appendChild(list);
      continue;
    }
    if (!line.trim()) { i += 1; continue; }
    const paragraph = [];
    while (i < lines.length && lines[i].trim() && !/^\s*```/.test(lines[i]) && !/^#{1,6}\s/.test(lines[i]) && !/^\s*([-*+]|\d+[.)])\s+/.test(lines[i])) {
      paragraph.push(lines[i]);
      i += 1;
    }
    const element = document.createElement("p");
    element.innerHTML = paragraph.map(inline).join("<br>");
    into.appendChild(element);
  }
}

function codeBlock(language, lines) {
  const wrap = document.createElement("div");
  wrap.className = "code";
  const head = document.createElement("div");
  head.className = "code-head";
  const label = document.createElement("span");
  label.textContent = language || "text";
  const copy = document.createElement("button");
  copy.className = "copy";
  copy.type = "button";
  copy.textContent = "Copy";
  copy.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      copy.textContent = "Copied";
    } catch (_) {
      copy.textContent = "Copy failed";
    }
    setTimeout(() => { copy.textContent = "Copy"; }, 1600);
  });
  head.append(label, copy);
  const pre = document.createElement("pre");
  const diff = language === "diff" || language === "patch";
  for (const line of lines) {
    const row = document.createElement("span");
    row.textContent = `${line}\n`;
    if (diff && line.startsWith("+") && !line.startsWith("+++")) row.className = "add";
    if (diff && line.startsWith("-") && !line.startsWith("---")) row.className = "del";
    pre.appendChild(row);
  }
  wrap.append(head, pre);
  return wrap;
}

// ---- conversation --------------------------------------------------------------------------
const GLYPH = '<svg viewBox="0 0 64 64" fill="none"><path fill="#1C1C1C" d="M15 15h34v34H15z"/><path d="M21 32h8m6 0h8" stroke="#76B900"/><path d="m32 27 1.7 3.3L37 32l-3.3 1.7L32 37l-1.7-3.3L27 32l3.3-1.7L32 27Z" fill="#24D046"/></svg>';

function scrollToEnd() {
  const near = conversation.scrollHeight - conversation.scrollTop - conversation.clientHeight < 160;
  if (near) conversation.scrollTop = conversation.scrollHeight;
}

function addEntry(kind, text) {
  const entry = document.createElement("div");
  entry.className = `entry ${kind}`;
  if (kind === "user" || kind === "directive") {
    const bubble = document.createElement("div");
    bubble.className = "bubble";
    bubble.textContent = text;
    entry.appendChild(bubble);
  } else if (kind === "assistant") {
    entry.innerHTML = `<span class="glyph" aria-hidden="true">${GLYPH}</span>`;
    const prose = document.createElement("div");
    prose.className = "prose";
    renderMarkdown(text, prose);
    entry.appendChild(prose);
  } else {
    entry.textContent = text;
  }
  conversation.appendChild(entry);
  scrollToEnd();
  return entry;
}

function startReply() {
  const entry = addEntry("assistant", "");
  const prose = entry.querySelector(".prose");
  prose.innerHTML = '<span class="typing" aria-label="Working"><span></span><span></span><span></span></span>';
  state.reply = { entry, prose, text: "", streamed: false, frame: 0 };
}

function streamReply(text, separator) {
  if (!state.reply) startReply();
  const reply = state.reply;
  if (separator) {
    if (reply.text) reply.text += "\n\n";
    return;
  }
  reply.text += text;
  reply.streamed = true;
  if (!reply.frame) {
    reply.frame = requestAnimationFrame(() => {
      reply.frame = 0;
      renderMarkdown(reply.text, reply.prose);
      scrollToEnd();
    });
  }
}

function setWorking(working) {
  state.working = working;
  if (working) {
    app.setAttribute("data-working", "");
    state.startedAt = Date.now();
  } else {
    app.removeAttribute("data-working");
  }
  updateSend();
  updateStatus();
}

// ---- status, models, sessions ---------------------------------------------------------------
function updateStatus() {
  const dot = $("#status-dot");
  const text = $("#status-text");
  let line;
  if (state.working) {
    const seconds = Math.floor((Date.now() - state.startedAt) / 1000);
    line = `Working ${seconds}s · Esc to stop`;
  } else if (state.provider && state.provider.type === "provider") {
    dot.dataset.state = "ready";
    line = `${state.provider.model} · ${state.provider.provider}`;
  } else if (state.provider) {
    dot.dataset.state = "offline";
    line = "Offline · no model replies";
  } else {
    dot.dataset.state = "";
    line = "Connecting to a model…";
  }
  text.textContent = line;
  $("#foot-connection").textContent = state.provider && state.provider.type === "provider" ? state.provider.model : line;
}

function renderModels() {
  const list = $("#models-list");
  list.textContent = "";
  const connection = $("#models-connection");
  if (!state.provider) { connection.textContent = "Connecting…"; return; }
  if (state.provider.type !== "provider") {
    connection.textContent = `No model provider: ${state.provider.reason}. Put OPENROUTER_API_KEY or NEBIUS_API_KEY in ~/.polaris/.env, or set local.model, then restart.`;
    $("#model-label").textContent = "No model";
    return;
  }
  connection.textContent = `${state.provider.model} · ${state.provider.provider}`;
  $("#model-label").textContent = shortModel(state.provider.model);
  $("#model-pill").title = state.provider.model;
  for (const [effort, model] of state.models) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "model-row";
    row.setAttribute("role", "listitem");
    row.setAttribute("aria-current", String(effort === state.effort));
    row.innerHTML = `<span class="effort"></span><span class="name"></span>`;
    row.querySelector(".effort").textContent = effort;
    row.querySelector(".name").textContent = model;
    row.addEventListener("click", () => {
      if (effort === state.effort) return;
      if (state.working) { showNotice("Change the model after the reply finishes.", 4000); return; }
      api("/api/effort", { effort });
    });
    list.appendChild(row);
  }
}

async function refreshSessions() {
  const response = await api("/api/sessions");
  if (!response.ok) return;
  state.sessions = await response.json();
  renderSessions();
}

function renderSessions() {
  const list = $("#session-list");
  const filter = $("#session-filter").value.trim().toLowerCase();
  list.textContent = "";
  const rows = state.sessions
    .filter((row) => !filter || (row.title || "Untitled session").toLowerCase().includes(filter))
    .sort((a, b) => b.updated_ms - a.updated_ms);
  if (!rows.length) {
    const empty = document.createElement("p");
    empty.className = "empty-list";
    empty.textContent = filter ? "No session matches." : "No sessions yet. Send a message to start one.";
    list.appendChild(empty);
    return;
  }
  const groups = new Map();
  for (const row of rows) {
    if (!groups.has(row.workspace)) groups.set(row.workspace, []);
    groups.get(row.workspace).push(row);
  }
  for (const [workspace, items] of groups) {
    const name = document.createElement("div");
    name.className = "group-name";
    name.textContent = folder(workspace);
    name.title = workspace;
    list.appendChild(name);
    for (const row of items) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "session-row";
      button.setAttribute("role", "listitem");
      button.setAttribute("aria-current", String(row.id === state.openId));
      button.innerHTML = '<span class="title"></span><span class="when"></span>';
      button.querySelector(".title").textContent = row.title || "Untitled session";
      button.querySelector(".when").textContent = relativeTime(row.updated_ms);
      button.addEventListener("click", () => {
        if (row.id === state.openId) return;
        if (state.working) { showNotice("A reply is still running; press Esc to stop it first.", 4000); return; }
        api("/api/open", { resume: row.id });
      });
      list.appendChild(button);
    }
  }
}

// ---- panels ----------------------------------------------------------------------------------
function closePanels(except) {
  for (const panel of document.querySelectorAll(".panel")) {
    if (panel.id !== except) panel.hidden = true;
  }
  for (const button of document.querySelectorAll("[data-panel]")) {
    button.setAttribute("aria-expanded", String(`panel-${button.dataset.panel}` === except && !$(`#${except}`).hidden));
  }
}

async function togglePanel(name) {
  const panel = $(`#panel-${name}`);
  const opening = panel.hidden;
  closePanels(opening ? panel.id : "");
  panel.hidden = !opening;
  for (const button of document.querySelectorAll(`[data-panel=${name}]`)) button.setAttribute("aria-expanded", String(opening));
  if (opening && name === "usage") {
    const lines = [
      `This session    ${state.usage.requests} requests`,
      `Prompt          ${state.usage.prompt_tokens} tokens`,
      `Completion      ${state.usage.completion_tokens} tokens`,
      "",
    ];
    $("#usage-lines").textContent = `${lines.join("\n")}\nLoading activity…`;
    const response = await api("/api/stats");
    if (response.ok) $("#usage-lines").textContent = `${lines.join("\n")}\n${(await response.json()).join("\n")}`;
  }
}

// ---- composer ----------------------------------------------------------------------------------
function updateSend() {
  sendButton.disabled = !state.working && (!textarea.value.trim() || state.pending !== null);
  sendButton.setAttribute("aria-label", state.working ? "Stop the reply" : "Send message");
}

function submit() {
  if (state.working) { api("/api/interrupt", {}); showNotice("Stopping…"); return; }
  const text = textarea.value;
  if (!text.trim() || state.pending) return;
  state.pending = { text, plan: state.plan };
  updateSend();
  showNotice("");
  if (state.openId) api("/api/submit", state.pending);
  else api("/api/open", {});
}

// ---- events from the server ------------------------------------------------------------------
function handle(event) {
  switch (event.type) {
    case "provider":
      state.provider = event;
      state.models = event.models;
      state.effort = event.effort;
      renderModels();
      updateStatus();
      break;
    case "provider-unavailable":
      state.provider = event;
      renderModels();
      updateStatus();
      break;
    case "model-switched":
      if (state.provider && state.provider.type === "provider") state.provider.model = event.model;
      state.effort = event.effort;
      renderModels();
      updateStatus();
      showNotice(`Model ${event.model} · ${event.effort} effort`, 4000);
      break;
    case "opening":
      conversation.textContent = "";
      state.reply = null;
      break;
    case "opened":
      // Also the first event of a replay after a reconnect: rebuild the conversation from here.
      conversation.textContent = "";
      state.reply = null;
      setWorking(false);
      state.openId = event.id;
      $("#session-title").textContent = event.title || "Untitled session";
      $("#foot-workspace").textContent = folder(event.workspace);
      state.usage = { requests: 0, prompt_tokens: 0, completion_tokens: 0 };
      setView("session");
      if (state.provider && state.provider.type !== "provider") addEntry("system", "Offline session · messages are recorded locally; no model replies.");
      if (state.pending) api("/api/submit", state.pending);
      refreshSessions();
      break;
    case "closed":
      state.openId = null;
      break;
    case "message":
      addEntry(event.kind, event.text);
      break;
    case "committed":
      if (state.pending && textarea.value === state.pending.text) textarea.value = "";
      state.pending = null;
      updateSend();
      refreshSessions();
      break;
    case "rejected":
      state.pending = null;
      updateSend();
      showNotice(`Not sent: ${event.reason}`);
      break;
    case "reply-started":
      setWorking(true);
      startReply();
      break;
    case "reply-text":
      streamReply(event.text, false);
      break;
    case "reply-step":
      streamReply("", true);
      break;
    case "reply-finished":
      if (state.reply && !state.reply.streamed) {
        if (event.answer && event.answer.trim()) renderMarkdown(event.answer, state.reply.prose);
        else { state.reply.entry.remove(); addEntry("warning", "The model stopped without an answer."); }
      } else if (state.reply) {
        renderMarkdown(state.reply.text, state.reply.prose);
      }
      state.reply = null;
      addEntry("system", event.summary);
      setWorking(false);
      if (notice.textContent === "Stopping…") showNotice("");
      break;
    case "reply-failed":
      if (state.reply && !state.reply.streamed) state.reply.entry.remove();
      state.reply = null;
      addEntry(event.reason === "Interrupted" ? "warning" : "error", event.reason === "Interrupted" ? "Stopped." : event.reason);
      setWorking(false);
      if (notice.textContent === "Stopping…") showNotice("");
      break;
    case "usage":
      state.usage = event;
      break;
    case "notice":
      showNotice(event.text, 8000);
      break;
    default:
      break;
  }
}

function connect() {
  const source = new EventSource(`/api/events?token=${encodeURIComponent(state.token)}`);
  source.onmessage = (message) => {
    try { handle(JSON.parse(message.data)); } catch (_) { /* malformed event */ }
  };
  source.onerror = () => {
    $("#status-text").textContent = "Reconnecting to Polaris…";
    $("#status-dot").dataset.state = "offline";
  };
  source.onopen = () => updateStatus();
}

// ---- star field: three seeded canvases instead of 175 KB of SVG ----------------------------
function drawStars() {
  const ratio = Math.min(window.devicePixelRatio || 1, 2);
  for (const canvas of document.querySelectorAll(".stars")) {
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    if (!width || !height) continue;
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
    const context = canvas.getContext("2d");
    if (!context) continue;
    let seed = Number(canvas.dataset.layer) * 2654435761;
    const random = () => {
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    const count = Math.round((width * height) / 3200);
    context.scale(ratio, ratio);
    for (let i = 0; i < count; i += 1) {
      const alpha = 0.25 + random() * 0.6;
      context.fillStyle = random() < 0.5 ? `rgba(167,177,158,${alpha})` : `rgba(181,193,173,${alpha})`;
      const size = random() < 0.08 ? 1.5 : 1;
      context.fillRect(random() * width, random() * height, size, size);
    }
  }
}

// ---- wiring --------------------------------------------------------------------------------------
function init() {
  // A new link (for example after `polaris web` restarted) carries a new token: start over.
  window.addEventListener("hashchange", () => { if (/token=/.test(location.hash)) location.reload(); });
  state.token = readToken();
  try { if (localStorage.getItem("polaris-reduce-motion") === "1") { app.setAttribute("data-reduce-motion", ""); $("#reduce-motion").checked = true; } } catch (_) { /* storage blocked */ }
  if (!state.token) showNotice("Open the link that `polaris web` printed; it carries this run's access token.");
  textarea.addEventListener("input", () => { updateSend(); if (notice.textContent && !state.pending) showNotice(""); });
  textarea.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) { event.preventDefault(); submit(); }
  });
  $("#composer").addEventListener("submit", (event) => { event.preventDefault(); submit(); });
  $("#mode-chip").addEventListener("click", () => {
    state.plan = !state.plan;
    $("#mode-chip").setAttribute("aria-pressed", String(state.plan));
    $("#mode-label").textContent = state.plan ? "Plan" : "Auto";
  });
  for (const button of document.querySelectorAll("[data-panel]")) {
    button.setAttribute("aria-expanded", "false");
    button.addEventListener("click", (event) => { event.stopPropagation(); togglePanel(button.dataset.panel); });
  }
  for (const button of document.querySelectorAll("[data-action=new]")) {
    button.addEventListener("click", () => {
      if (state.working) { showNotice("A reply is still running; press Esc to stop it first.", 4000); return; }
      if (state.openId) api("/api/close", {});
      state.openId = null;
      conversation.textContent = "";
      $("#session-title").textContent = "New session";
      setView("welcome");
      textarea.focus();
    });
  }
  for (const button of document.querySelectorAll("[data-action=sessions]")) {
    button.addEventListener("click", () => { setView("session"); refreshSessions(); });
  }
  $("#session-filter").addEventListener("input", renderSessions);
  $("#reduce-motion").addEventListener("change", (event) => {
    const on = event.target.checked;
    if (on) app.setAttribute("data-reduce-motion", ""); else app.removeAttribute("data-reduce-motion");
    try { localStorage.setItem("polaris-reduce-motion", on ? "1" : "0"); } catch (_) { /* storage blocked */ }
  });
  document.addEventListener("click", (event) => { if (!event.target.closest(".panel")) closePanels(""); });
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    if (state.working) { api("/api/interrupt", {}); showNotice("Stopping…"); return; }
    closePanels("");
  });
  let resizeTimer = 0;
  window.addEventListener("resize", () => { clearTimeout(resizeTimer); resizeTimer = setTimeout(drawStars, 150); });
  setInterval(() => { if (state.working) updateStatus(); }, 1000);
  drawStars();
  updateSend();
  if (state.token) { connect(); refreshSessions(); }
  textarea.focus();
}

init();
