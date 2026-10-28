"""Render the checked-in, dependency-free Polaris HTML/CSS design preview.

Run normally to regenerate pages; --check verifies output without editing files.
"""
from __future__ import annotations
import argparse
from datetime import date, timedelta
from html import escape
import json
import random
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WEB = ROOT / "webui"
ICONS = json.loads((WEB / "assets/icons.json").read_text(encoding="utf-8"))


def icon(name: str, extra: str = "") -> str:
    return f'<svg class="icon {extra}" aria-hidden="true" viewBox="0 0 24 24">{ICONS[name]}</svg>'


def button(text: str, href: str, primary: bool = False, name: str = "arrow") -> str:
    return f'<a class="button{ " primary" if primary else ""}" href="{href}">{escape(text)}{icon(name)}</a>'


NAV = [("chat", "chat", "Chat"), ("history", "history", "History"), ("models", "grid", "Models"),
       ("usage", "chart", "Usage"), ("states", "layers", "Interface states"), ("settings", "sliders", "Settings")]


def navigation(page: str) -> str:
    return ''.join(f'<a class="nav-link" href="{key}.html" aria-label="{label}"{ " aria-current=\"page\"" if key == page else ""}>{icon(symbol)}<span class="nav-tooltip">{label}</span></a>' for key, symbol, label in NAV)

def preview_info(extra: str = "") -> str:
    return f'''<details class="preview-info {extra}"><summary>{icon('spark')}<span>Design preview</span></summary><div class="preview-description"><h2>Polaris design preview</h2><p>HTML and CSS only. Conversations, timers, context, and usage are labeled samples.</p><p>Editing and native controls are available. Sending, uploads, inference, and saving are not connected.</p><a href="states.html">Explore interface states{icon('arrow')}</a></div></details>'''


def sidebar(page: str) -> str:
    return f'''<aside class="sidebar" aria-label="Workspace navigation">
<a class="brand" href="index.html" aria-label="Polaris home"><img src="assets/polaris.svg" alt="" width="28" height="28"></a>
<a class="new-chat" href="index.html" aria-label="New conversation">{icon('plus')}<span class="nav-tooltip">New conversation</span></a>
<nav class="nav-group" aria-label="Main">{navigation(page)}</nav>
{preview_info()}</aside>'''

def context() -> str:
    return f'''<details class="context-toggle"><summary aria-label="Session context">{icon('context')}<span class="context-label">Context</span></summary>
<aside class="context-aside" aria-label="Session context"><div class="section-head"><h2>Session context</h2><span class="badge">sample</span></div><p class="panel-intro">The useful things, close at hand.</p>
<h3>Workspace files</h3><div class="context-file">{icon('file')}src/validation.rs</div><div class="context-file">{icon('file')}src/handlers.rs</div><div class="context-file">{icon('file')}tests/requests.rs</div>
<h3>Current goal</h3><p class="blue" style="font-size:12px">Extract the shared request guard</p><p class="mono muted" style="margin-top:8px">active · 1m 24s · sample</p>
<h3>Session settings</h3><div class="setting-row"><span class="blue">$websearch</span><span>off</span></div><div class="setting-row"><span class="blue">$subagents</span><span>off</span></div><div class="setting-row"><span class="blue">$skills</span><span>[review]</span></div><div class="setting-row"><span class="blue">$rag</span><span>[workspace]</span></div><div class="setting-row"><span class="blue">$links</span><span>[]</span></div>
<p class="page-note">Sample context. No files or services are connected.</p><a class="button small quiet" href="settings.html#session" style="margin-top:18px">Session settings{icon('arrow')}</a></aside></details>'''


def header(title: str, page: str) -> str:
    return f'''<header class="topbar"><details class="mobile-menu"><summary aria-label="Open navigation">{icon('menu')}</summary><nav aria-label="Mobile navigation"><a class="nav-link" href="index.html" aria-label="New conversation">{icon('plus')}<span>New conversation</span></a>{navigation(page)}</nav></details>
{preview_info('mobile-preview')}<div class="top-actions">{context()}</div></header>'''

def shell(title: str, page: str, content: str) -> str:
    return f'''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="dark"><meta name="description" content="Polaris — a focused coding workspace. Static HTML and CSS design preview."><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:; form-action 'none'; base-uri 'none'"><title>{escape(title)} · Polaris</title><link rel="icon" href="assets/polaris.svg" type="image/svg+xml"><link rel="stylesheet" href="styles.css"></head>
<body class="page-{page}"><a class="skip-link" href="#main">Skip to content</a><div class="app">{sidebar(page)}<div class="workspace">{header(title,page)}<main id="main" class="content-flow">{content}</main></div></div></body></html>
'''


def page_heading(kicker: str, title: str, description: str, action: str = "") -> str:
    return f'<div class="page-heading"><h1>{title}</h1>{action}</div>'

def model_menu(prefix: str) -> str:
    models=[('super','Nemotron Super','Nemotron Super 49B v1.5'),('nano9','Nemotron Nano 9B','Nemotron Nano 9B v2'),('nano8','Nemotron Nano 8B','Nemotron Nano 8B v1')]
    labels=''.join(f'<span class="model-label model-label-{key}">{short}</span>' for key,short,_ in models)
    rows=''.join(f'<label class="menu-model-row"><input class="menu-radio" type="radio" name="{prefix}-model" value="{key}"{ " checked" if i==0 else ""}><span>{full}</span>{icon("check","selection-check")}</label>' for i,(key,_,full) in enumerate(models))
    modes=''.join(f'<label><input class="menu-radio" type="radio" name="{prefix}-mode" value="{mode}"{ " checked" if mode=="plan" else ""}><span class="{color}">{mode}</span></label>' for mode,color in [('plan','gold'),('auto','pink')])
    efforts=''.join(f'<label><input class="menu-radio" type="radio" name="{prefix}-effort" value="{level}"{ " checked" if level=="high" else ""}><span>{level}</span></label>' for level in ['low','medium','high','xhigh','max'])
    return f'''<details class="composer-popover"><summary class="composer-pill" aria-controls="{prefix}-options"><span class="sr-only">Model, mode, and effort settings: </span><span class="model-short">{labels}</span>{icon('chevron','menu-chevron')}</summary><div class="popover-body" id="{prefix}-options"><fieldset class="menu-models"><legend>Model</legend>{rows}</fieldset><div class="menu-settings"><fieldset class="menu-setting"><legend class="sr-only">Interaction mode</legend><span class="menu-setting-label" aria-hidden="true">Mode</span><div class="mode-options">{modes}</div></fieldset><fieldset class="menu-setting effort-group"><legend class="sr-only">Reasoning effort</legend><span class="menu-setting-label" aria-hidden="true">Effort</span><div class="effort-options">{efforts}</div></fieldset></div><footer class="menu-footer"><a href="models.html">Model settings{icon('arrow')}</a><small>Preview only · not saved</small></footer></div></details>'''


def composer(prefix: str = "draft", goal: bool = False, attachments: bool = False) -> str:
    goal_html = f'<div class="goal-line">{icon("goal")}<span>goal active</span><span class="dot"></span><span>1m 24s · sample</span></div>' if goal else ''
    chips = f'<div class="chip-row"><a class="attachment-chip" href="states.html#attachments">{icon("image")}[Image 1]</a><a class="attachment-chip" href="states.html#attachments">{icon("file")}[Pasted Content 684 chars]</a></div>' if attachments else ''
    return f'''<div class="composer-wrap">{goal_html}<section class="composer glass" aria-label="Message composer">{chips}<div class="composer-input"><button class="icon-button attach-button" type="button" disabled aria-label="Attach files — unavailable in design preview" title="Uploads are not connected">{icon('plus')}</button><label class="sr-only" for="{prefix}-text">Message for Polaris</label><textarea id="{prefix}-text" placeholder="Ask Polaris anything…" spellcheck="false" rows="1"></textarea>
{model_menu(prefix)}<button class="send-button" type="button" disabled aria-label="Send message — inference is unavailable in this preview" title="Design preview — sending is not connected">{icon('up')}</button></div></section></div>'''


def chat_composer(prefix: str = "chat", attachments: bool = False) -> str:
    chips = f'<div class="chip-row"><a class="attachment-chip" href="states.html#attachments">{icon("image")}[Image 1]</a><a class="attachment-chip" href="states.html#attachments">{icon("file")}[Pasted Content 684 chars]</a></div>' if attachments else ''
    return f'''<div class="composer-wrap chat-draft-wrap"><div class="goal-line">{icon('goal')}<span>goal active</span><span class="dot"></span><span>1m 24s · sample</span></div><section class="composer chat-draft" aria-label="Message composer">{chips}<label class="sr-only" for="{prefix}-text">Message for Polaris</label><textarea id="{prefix}-text" placeholder="What’s next?" spellcheck="false" rows="1"></textarea><div class="composer-input chat-draft-toolbar"><button class="chat-capsule chat-attach" type="button" disabled aria-label="Attach files — unavailable in design preview" title="Uploads are not connected">{icon('plus')}<span>Attach</span></button><div class="chat-draft-actions">{model_menu(prefix)}<button class="chat-capsule chat-send" type="button" disabled aria-label="Send message — inference is unavailable in this preview" title="Design preview — sending is not connected">{icon('up')}</button></div></div></section><p class="chat-draft-note">Design preview · sending and uploads are not connected</p></div>'''


def artifact(title: str, subtitle: str, filename: str, symbol: str = "file", gold: bool = False) -> str:
    download_name = title if title.endswith((".md", ".txt", ".diff")) else filename
    return f'''<article class="chat-file"><span class="chat-file-icon{ ' gold' if gold else ''}">{icon(symbol)}</span><div class="chat-file-copy"><strong>{escape(title)}</strong><small>{escape(subtitle)}</small></div><a class="chat-capsule chat-download" href="downloads/{filename}" download="{escape(download_name)}" aria-label="Download {escape(title)} — design sample">{icon('download')}<span>Download</span></a></article>'''


def chat_tool(sample: bool = False) -> str:
    copy = '<p>Sample tool output. No workspace files were read.</p><p>Both handlers repeat an empty-input check.</p><p>The shared guard can return a typed validation error.</p>' if not sample else '<p>Sample tool details expand here.</p><p>Only this activity changes when its title is opened.</p>'
    return f'<details class="activity"><summary><span class="inline-bullet"></span><span>Read <span class="tool-path">· src/validation.rs</span></span>{icon("chevron","activity-chevron")}</summary><div class="tool-body">{copy}</div></details>'


def code_block(lines: list[str], language: str = "rust", filename: str = "src/validation.rs", diff: bool = False) -> str:
    rendered = ''
    for i, line in enumerate(lines, 1):
        kind = ' removed' if diff and line.startswith('-') else ' added' if diff and line.startswith('+') else ''
        rendered += f'<span class="code-line{kind}"><span class="line-number" aria-hidden="true">{i}</span>{line}</span>'
    return f'<div class="code-block"><div class="code-head"><span>{icon("code")} &nbsp;{filename}</span><span>{language}</span></div><pre><code>{rendered}</code></pre></div>'


def welcome_page() -> str:
    stars=''.join(f'<span class="star-layer star-layer-{i}"></span>' for i in range(1,4))
    return shell('Your workspace','index',f'''<section class="welcome"><div class="welcome-atmosphere" aria-hidden="true"><span class="atmosphere-light atmosphere-mesh"></span><span class="atmosphere-light atmosphere-nebula"></span><span class="atmosphere-light atmosphere-aurora aurora-one"></span><span class="atmosphere-light atmosphere-aurora aurora-two"></span><span class="atmosphere-grain"></span>{stars}</div><div class="welcome-stage"><h1>What’s on your mind for Polaris?</h1>{composer('welcome')}</div></section>''')

def chat_page() -> str:
    lines = ['<span class="syn-keyword">pub fn</span> <span class="syn-function">require_input</span>(input: &amp;str) -&gt; Result&lt;(), Error&gt; {', '    <span class="syn-keyword">if</span> input.trim().is_empty() {', '        <span class="syn-keyword">return</span> Err(Error::EmptyInput);', '    }', '    Ok(())', '}']
    return shell('Request validation refactor','chat',f'''<div class="chat-layout"><header class="chat-heading"><div><p class="chat-eyebrow">Sample conversation</p><h1>Request validation</h1></div><a class="chat-capsule chat-new" href="index.html">{icon('plus')}<span>New chat</span></a></header><div class="chat-transcript"><section class="chat-prompt" aria-label="Your message"><p>Extract the repeated validation into a shared helper. Keep error responses at the handler boundary.</p></section><article class="chat-response" aria-label="Polaris response">{chat_tool()}<h2>One guard. Clear boundaries.</h2><p>Both handlers can use the same small helper. Keep validation in <code>require_input</code> and build each HTTP error response at the handler boundary.</p>
{code_block(lines)}<p>The helper knows whether the input is valid. Each route decides how to explain a failure.</p><ol class="chat-steps"><li>Extract the guard and return a typed error.</li><li>Map the error at each handler boundary.</li><li>Cover missing, whitespace-only, and valid input.</li></ol><section class="chat-files" aria-label="Downloadable design samples"><h3>Files for the next step</h3>{artifact('workspace-plan.md','Implementation notes · design sample','workspace-plan.md','file',True)}{artifact('validation.diff','Shared request guard · design sample','validation.diff','code')}</section><footer class="chat-completion" id="latest"><span class="chat-completion-mark">{icon('check')}</span><span>Worked for 14s</span><span class="chat-completion-time">11:42 · sample</span></footer></article></div><div class="chat-dock">{chat_composer()}</div></div>''')


def models_page() -> str:
    cards=''
    for i,(name,desc,meta) in enumerate([
        ('Nemotron Super 49B v1.5','Reasoning and tool calling','NVIDIA NIM · balanced capability'),
        ('Nemotron Nano 9B v2','Compact reasoning and agentic tasks','NVIDIA NIM · compact model'),
        ('Nemotron Nano 8B v1','Reasoning for PC and edge','NVIDIA NIM · edge-oriented model')]):
        cards+=f'<label class="model-card"><input type="radio" name="active-model" value="{i}"{ " checked" if i==0 else ""}><span><span class="model-name">{name}</span><span class="model-description">{desc}</span></span></label>'
    efforts=''.join(f'<label><input type="radio" name="effort" value="{level}"{ " checked" if level=="high" else ""}>{level}</label>' for level in ['low','medium','high','xhigh','max'])
    return shell('Models & effort','models',f'''<div class="page">{page_heading('','Models &amp; effort','')}
<section class="panel"><div class="section-head"><h2>Provider</h2></div><div class="field"><label for="provider">Model provider</label><select id="provider"><option>NVIDIA NIM</option><option>Anthropic</option><option>OpenAI</option><option>Local</option></select></div><h2 style="margin-top:28px">Available models</h2><div class="model-list">{cards}</div>
<fieldset class="effort-group" id="effort"><legend>Reasoning effort</legend><div class="effort-options">{efforts}</div></fieldset></section><details class="explanation"><summary>About these controls{icon("chevron")}</summary><p>Sample model catalog. Selections are local visual previews; providers are not connected and preferences are not saved.</p></details></div>''')


def history_page() -> str:
    rows=''
    for symbol,title,desc,href,tag in [('code','Request validation refactor','Shared guards and typed errors.','chat.html','Rust'),('spark','A fresh workspace','An empty draft.','index.html','New'),('goal','Planning with goals','Active and paused goal examples.','states.html#goals','States')]:
        rows+=f'<a class="history-card" href="{href}">{icon(symbol)}<span class="history-copy"><strong>{title}</strong><p>{desc}</p></span>{icon("arrow")}</a>'
    return shell('Conversation history','history',f'''<div class="page">{page_heading('','History','',button('New conversation','index.html',False,'plus'))}<p class="sample-caption">Sample conversations · no saved history</p><section class="history-group" aria-label="Sample conversations">{rows}</section></div>''')

def switch_row(name: str,title: str,desc: str,checked: bool = False,disabled: bool = False) -> str:
    return f'<label class="setting-control" for="{name}"><span><strong>{title}</strong><small>{desc}</small></span><input class="switch" type="checkbox" id="{name}"{ " checked" if checked else ""}{ " disabled" if disabled else ""}></label>'


def settings_page() -> str:
    swatches=''.join(f'<span class="swatch" style="--swatch:{color}" title="{name}" role="img" aria-label="{name}"></span>' for name,color in [('Polaris green','#76b900'),('Plan gold','#f9e2af'),('Goal blue','#89b4fa'),('Auto magenta','#f5c2e7')])
    return shell('Workspace settings','settings',f'''<div class="page">{page_heading('Make room for your way of working','Settings','A familiar set of preferences, with a quieter place for the details. Native controls are local to this page and are not saved.')}<nav class="settings-nav" aria-label="Settings sections"><a href="#appearance">Appearance</a><a href="#interaction">Interaction</a><a href="#session">Session settings</a></nav>
<section class="panel" id="appearance"><div class="section-head"><h2>Appearance</h2>{icon('moon')}</div><div class="setting-control"><span><strong>Polaris dark</strong><small>Black canvas. Graphite controls. Polaris accents.</small></span><span class="swatches">{swatches}</span></div>{switch_row('high-contrast','Stronger contrast','Increase border and secondary-text contrast on this page.')}{switch_row('larger-text','Larger text','Increase the page base text size; does not persist.')}{switch_row('reduce-motion','Reduce motion','Disable hover motion and smooth scrolling on this page. System preferences are also respected.')}<p class="page-note">Space Grotesk · Inter · JetBrains Mono. All fonts are bundled locally.</p></section>
<section class="panel" id="interaction"><div class="section-head"><h2>Interaction</h2>{icon('keyboard')}</div><div class="setting-control"><span><strong>Native text editing</strong><small>Selection, typing, paste, and line breaks use the browser textarea.</small></span><span class="badge">available</span></div><div class="setting-control"><span><strong>Send with Enter</strong><small>Sending and custom keymaps need a future runtime. Enter currently inserts a newline.</small></span><span class="badge">not connected</span></div><div class="setting-control"><span><strong>File and command completion</strong><small>Static appearance examples are available in the state library.</small></span><a class="button small quiet" href="states.html#suggestions">Preview{icon('arrow')}</a></div></section>
<details class="panel settings-disclosure"><summary><h2>Session settings</h2>{icon("chevron")}</summary><div class="disclosure-content" id="session"><p class="panel-intro">The CLI’s dollar settings, translated into a small, readable form. These fields do not enable services.</p>{switch_row('web-search','Web search','$websearch · Use the web as a source of context.')}{switch_row('subagents','Subagents','$subagents · Allow parallel assistance.')}<div class="form-grid"><div class="field"><label for="agent-models">Subagent models <span class="mono blue">$subagents-model</span></label><input id="agent-models" value="[nemotron-super(low-xhigh)]" spellcheck="false"><small>Model labels and effort ranges are sample values.</small></div><div class="field"><label for="session-skills">Skills <span class="mono blue">$skills</span></label><input id="session-skills" value="[review]" spellcheck="false"></div><div class="field"><label for="rag">Retrieval sources <span class="mono blue">$rag</span></label><input id="rag" value="[workspace]" spellcheck="false"></div><div class="field"><label for="links">Reference links <span class="mono blue">$links</span></label><input id="links" value="[]" spellcheck="false"></div></div><div class="settings-actions"><button class="button primary" type="button" disabled title="Persistence is not implemented">Save preferences{icon('check')}</button></div></div></details></div>''')


def compact(number: int) -> str:
    return f'{number/1_000_000:.2f}M' if number >= 1_000_000 else f'{number/1000:.1f}k' if number >= 1000 else str(number)


def usage_data() -> list[dict]:
    start = date(2025, 10, 20)
    result = []
    for i in range(364):
        score = (i * 17 + (i // 7) * 13) % 23
        sessions = 0 if score < 7 else 1 + score % 5
        tokens = sessions * (3200 + (i * 7919) % 36000)
        result.append(dict(day=start+timedelta(days=i),sessions=sessions,tokens=tokens))
    return result


def usage_page() -> str:
    data = usage_data()
    total = sum(row['tokens'] for row in data)
    sessions = sum(row['sessions'] for row in data)
    active = sum(row['sessions'] > 0 for row in data)
    best = max(data,key=lambda row: row['sessions'])
    streak = longest = 0
    for row in data:
        streak = streak + 1 if row['sessions'] else 0
        longest = max(longest,streak)
    cells = ''.join(f'<span class="day" data-level="{min(4,row["sessions"])}" title="{row["day"].isoformat()}: {row["sessions"]} sample sessions"></span>' for row in data)
    colors=['#87ba34','#89b4fa','#f9e2af']
    months = ''.join(f'<span>{month}</span>' for month in ['Oct','Nov','Dec','Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct'])
    heat = f'<div class="calendar-months" aria-hidden="true">{months}</div><div class="calendar-wrap"><div class="weekdays" aria-hidden="true"><span>Mon</span><span>Wed</span><span>Fri</span></div><div class="heatmap" role="img" aria-label="Sample activity from October 20, 2025 to October 18, 2026. {active} active days and {sessions} sessions.">{cells}</div></div><div class="heat-legend">Less '+''.join(f'<i style="background:{c}" aria-hidden="true"></i>' for c in ['#252d1e','#384f20','#4d7224','#679e15','#87ba34'])+' More</div>'
    weekly=[sum(row['tokens'] for row in data[i:i+7])/7 for i in range(0,364,7)]
    peak=max(weekly)*.8
    paths=''
    for weight,color in zip([.75,.18,.07],colors):
        points=' '.join(f'{55+i*625/51:.1f},{210-value*weight/peak*175:.1f}' for i,value in enumerate(weekly))
        paths+=f'<polyline points="{points}" fill="none" stroke="{color}" stroke-width="1.7" stroke-linejoin="round"/>'
    grid=''.join(f'<line x1="55" y1="{210-i*43.75}" x2="680" y2="{210-i*43.75}" stroke="#ffffff12"/><text x="43" y="{213-i*43.75}" text-anchor="end">{compact(int(peak*i/4))}</text>' for i in range(5))
    chart=f'<svg class="chart" viewBox="0 0 710 245" role="img" aria-labelledby="chart-title chart-desc"><title id="chart-title">Sample tokens per day by model</title><desc id="chart-desc">Weekly daily averages from the labeled sample dataset. Nemotron Super has 75 percent, Nano 9B 18 percent, and Nano 8B 7 percent.</desc>{grid}{paths}<text x="55" y="235">Oct 2025</text><text x="350" y="235">Apr 2026</text><text x="626" y="235">Oct 2026</text></svg>'
    names=['Super 49B','Nano 9B','Nano 8B']
    legend='<div class="chart-legend">'+''.join(f'<span><i style="--series:{c}"></i>Nemotron {n}</span>' for n,c in zip(names,colors))+'</div>'
    breakdown='<div class="model-breakdown">'+''.join(f'<div><strong style="color:{c}">Nemotron {n}</strong><p>{int(w*100)}% of sample tokens<br>{compact(round(total*w))} tokens<br>Weekly daily average</p></div>' for n,c,w in zip(names,colors,[.75,.18,.07]))+'</div>'
    stats=''.join(f'<div class="stat"><span class="stat-label">{label}</span><div class="stat-value">{value}</div><div class="stat-meta">{note}</div></div>' for label,value,note in [('Total tokens',compact(total),'across 3 sample models'),('Sessions',str(sessions),'sample conversations'),('Active days',str(active),'of 364 calendar days'),('Longest streak',f'{longest} days','from the sample fixture')])
    return shell('Usage & activity','usage',f'''<div class="page">{page_heading('A little perspective','Usage &amp; activity','A calendar of the work, and a clearer view of what powers it. All figures below come from a deterministic design fixture.')}<div class="section-head"><span class="badge">DEMO DATA · NOT ACCOUNT USAGE</span><span class="mono muted" style="font-size:9px">Oct 2025 — Oct 2026</span></div><section class="usage-tabs" aria-label="Usage views"><div class="tab-switches"><label><input id="usage-overview" type="radio" name="usage-view" checked>Overview</label><label><input id="usage-models" type="radio" name="usage-view">Models</label></div><div class="usage-range"><strong>All time</strong><button type="button" disabled title="Date-range filtering is not implemented in this static preview">Last 7 days</button><button type="button" disabled title="Date-range filtering is not implemented in this static preview">Last 30 days</button></div><div class="usage-view overview-view"><section class="panel"><div class="section-head"><h2>Activity</h2><span class="kicker">52 weeks</span></div>{heat}<div class="usage-summary"><span>Favorite model<strong>Nemotron Super 49B</strong></span><span>Current streak<strong>{streak} days</strong></span><span>Most active day<strong>{best['day'].strftime('%b %d')} · {best['sessions']} sessions</strong></span><span>Source<strong>Design sample</strong></span></div></section>{ '<div class="stat-grid">'+stats+'</div>'}</div><div class="usage-view models-view"><section class="panel"><div class="section-head"><h2>Tokens per day</h2><span class="kicker">weekly average</span></div>{chart}{legend}{breakdown}</section></div></section></div>''')


def states_page() -> str:
    section_links=''.join(f'<a href="#{key}">{name}</a>' for key,name in [('activity','Activity'),('messages','Messages'),('goals','Goals'),('attachments','Attachments'),('suggestions','Suggestions'),('controls','Controls'),('drafts','Composer')])
    tool=chat_tool(sample=True)
    commands=''.join(f'<a class="suggestion{ " selected" if i==0 else ""}" href="{href}"><span>{name}</span><small>{desc}</small></a>' for i,(name,desc,href) in enumerate([('/model','Choose provider, model and effort','models.html'),('/settings','Make room for your preferences','settings.html'),('/usage','Review sample activity','usage.html'),('/sessions','Browse the sample history','history.html')]))
    files=''.join(f'<div class="suggestion file"><span class="token-mention">@{name}</span><small>{desc}</small></div>' for name,desc in [('validation.rs','src/validation.rs'),('handlers.rs','src/handlers.rs'),('requests.rs','tests/requests.rs')])
    dollars=''.join(f'<a class="suggestion dollar" href="settings.html#session"><span class="token-setting">{name}</span><small>{desc}</small></a>' for name,desc in [('$websearch','Use web sources'),('$subagents','Parallel assistance'),('$skills','Skills for this session'),('$rag','Retrieval sources'),('$links','Reference links')])
    diff=code_block(['-    !input.trim().is_empty()','+    require_input(input)'],'diff','src/handlers.rs · design sample',True)
    return shell('Interface states','states',f'''<div class="page">{page_heading('The details make the interface','Interface states','A static library of the states around a conversation. Native disclosures, links, downloads, textareas, and CSS choices can be explored.')}<nav class="state-index" aria-label="Jump to state section">{section_links}</nav>
<details class="state-section"><summary><h2>Activity</h2>{icon("chevron")}</summary><div class="state-content" id="activity"><div class="state-grid"><div class="state-box"><span class="kicker">Working · static snapshot</span><div class="working-line"><i></i>Working <span class="muted">(42s · esc to interrupt)</span></div><p class="page-note">Elapsed time and interruption are visual examples.</p></div><div class="state-box"><span class="kicker">Completed · static snapshot</span><div class="worked-line">Worked for 3m 36s · 01:47</div><p class="page-note">Quiet metadata after the response.</p></div><div class="state-box"><span class="kicker">Stopped</span><div class="working-line">{icon('stop')}Response stopped</div><p class="page-note">Your draft stays ready for the next step.</p></div><div class="state-box"><span class="kicker">Tool details · click to expand</span>{tool}</div></div></div></details>
<details class="state-section"><summary><h2>Messages & code</h2>{icon("chevron")}</summary><div class="state-content" id="messages"><div class="state-box"><div class="status-message"><strong>{icon('chat')}Ordinary message</strong><p>The request guard can move into a shared helper.</p></div><div class="status-message warning" style="margin-top:14px"><strong>{icon('warning')}Warning</strong><p>This sample change needs a closer look before applying.</p></div><div class="status-message error" style="margin-top:14px"><strong>{icon('warning')}Error</strong><p>Selected model is at capacity. Please try a different model.</p></div></div>{diff}<p class="page-note">Message labels and diff markers retain meaning without color. Code gutters are excluded from selection.</p></div></details>
<details class="state-section"><summary><h2>Goals</h2>{icon("chevron")}</summary><div class="state-content" id="goals"><div class="state-grid"><div class="state-box state-goal"><span class="kicker">Active goal · sample duration</span><div class="goal-line">{icon('goal')}goal active <span class="dot"></span>1m 24s</div><p class="page-note">Extract the shared validation guard.</p></div><div class="state-box state-goal"><span class="kicker">Paused goal · frozen sample duration</span><div class="goal-line">{icon('goal')}goal paused <span class="dot"></span>1m 24s</div><p class="page-note">The timer remains quiet while work is paused.</p></div></div></div></details>
<details class="state-section"><summary><h2>Artifacts & attachments</h2>{icon("chevron")}</summary><div class="state-content" id="attachments">{artifact('Request validation plan','MD · Downloadable design sample','workspace-plan.md','file',True)}{artifact('Shared request guard','DIFF · Downloadable design sample','validation.diff','code')}<p class="kicker" style="margin-top:24px">Hover appearance</p>{artifact('Request validation plan','MD · Hover appearance · design sample','workspace-plan.md').replace('chat-file','chat-file preview-hover',1)}<div class="chip-row" style="margin-top:20px"><a class="attachment-chip" href="#image-preview">{icon('image')}[Image 1]</a><a class="attachment-chip" href="#paste-preview">{icon('file')}[Pasted Content 684 chars]</a></div><details class="attachment-preview" id="image-preview"><summary class="image-summary">{icon('image')}Image 1 · Polaris mark{icon('chevron')}</summary><img src="assets/polaris.svg" alt="Thin gray Polaris chip with green North Star"><p class="page-note">A bundled image fixture. No image was uploaded.</p></details><details class="attachment-preview" id="paste-preview"><summary class="image-summary">{icon('file')}Pasted content · text fixture{icon('chevron')}</summary><pre>{escape((WEB / 'downloads/pasted-content.txt').read_text(encoding='utf-8'))}</pre><a class="button small quiet" href="downloads/pasted-content.txt" download>Download pasted content{icon('download')}</a></details><p class="long-sample"><span class="kicker">Long-filename wrapping</span></p>{artifact('polaris-request-validation-and-handler-boundary-review-notes-for-the-next-iteration.md','MD · Long-name layout example','workspace-plan.md')}
</div></details>
<details class="state-section"><summary><h2>Suggestions</h2>{icon("chevron")}</summary><div class="state-content" id="suggestions"><div class="state-grid"><div><p class="kicker" style="margin-bottom:12px">Slash commands · real preview links</p><div class="suggestions glass">{commands}</div></div><div><p class="kicker" style="margin-bottom:12px">File mentions · static list</p><div class="suggestions glass">{files}</div></div></div><div style="margin-top:24px"><p class="kicker" style="margin-bottom:12px">Session settings · opens the settings preview</p><div class="suggestions glass">{dollars}</div></div><div class="sample-tokens" style="margin-top:24px">Review <span class="token-mention">@src/validation.rs</span> with <span class="token-command">/plan</span><br><span class="token-setting">$websearch $skills=[review] $rag=[workspace]</span></div><p class="page-note">Live token highlighting and suggestion filtering need a future runtime. Hover and selection styling can be inspected here.</p></div></details>
<details class="state-section"><summary><h2>Controls</h2>{icon("chevron")}</summary><div class="state-content" id="controls"><div class="state-box"><div class="control-states"><a class="chat-capsule" href="#drafts">Continue{icon('arrow')}</a><a class="chat-capsule" href="#drafts">Default</a><a class="chat-capsule preview-hover" href="#drafts" aria-label="Hover appearance example; jump to composer">Hover</a><a class="chat-capsule preview-focus" href="#drafts" aria-label="Focus appearance example; jump to composer">Focus</a><button class="chat-capsule" type="button" disabled>Disabled</button></div><div style="text-align:center;margin-top:30px"><a class="latest-link" href="#latest-message">{icon('up')}To latest</a></div></div><p class="page-note">Hover and Focus examples hold those visual states for comparison. The actual keyboard ring is also available on every enabled control.</p></div></details>
<details class="state-section"><summary><h2>Composer</h2>{icon("chevron")}</summary><div class="state-content" id="drafts">{chat_composer('state',attachments=True)}<p class="page-note">The native textarea supports immediate editing, selection, paste, and multiline text. Sending and uploads are intentionally unavailable.</p><div class="message-anchor" id="latest-message" style="margin-top:40px"><span class="kicker">Latest message · anchor destination</span><p class="muted" style="margin-top:12px;font-size:12px">You’re at the latest preview section.</p></div></div></details></div>''')


def render_stars() -> dict[str, str]:
    """Rectangular, repeatable dust fields: scattered points plus uneven clusters."""
    rng=random.Random(21019)
    layers={}
    used=set()
    clusters=[(230,180,150,100),(1120,260,240,160),(650,480,130,85),
              (900,760,240,130),(1400,870,180,100),(120,670,100,180)]
    for layer in range(1,4):
        points=[]
        while len(points)<800:
            if rng.random()<.65:
                x=rng.randrange(1600);y=rng.randrange(1000)
            else:
                cx,cy,sx,sy=rng.choice(clusters)
                x=round(rng.gauss(cx,sx));y=round(rng.gauss(cy,sy))
            if not (0<=x<1600 and 0<=y<1000) or (x,y) in used:
                continue
            used.add((x,y))
            color=rng.choice(['#b5c1ad','#a7b19e','#819b70','#d0d3cb'])
            opacity=rng.choice(['.25','.35','.5','.65','.8'])
            points.append(f'<rect x="{x}" y="{y}" width="1" height="1" fill="{color}" opacity="{opacity}"/>')
        layers[f'assets/stars-{layer}.svg']='<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="1000" viewBox="0 0 1600 1000">'+''.join(points)+'</svg>\n'
    return layers


def render_atmosphere_assets() -> dict[str, str | bytes]:
    # A small, immutable noise tile: no fullscreen live filter or runtime work.
    rng=random.Random(50822)
    raw=b''.join(b'\x00'+bytes(max(0,min(255,round(rng.gauss(128,52)))) for _ in range(128)) for _ in range(128))
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack('!I',len(data))+kind+data+struct.pack('!I',zlib.crc32(kind+data)&0xffffffff)
    grain=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('!IIBBBBB',128,128,8,0,0,0,0))+chunk(b'IDAT',zlib.compress(raw,9))+chunk(b'IEND',b'')
    ribbon='''<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="1000" viewBox="0 0 1600 1000"><defs><linearGradient id="light" x1="0" y1="0" x2="1" y2=".2"><stop stop-color="#53795a" stop-opacity="0"/><stop offset=".22" stop-color="#648e65" stop-opacity=".28"/><stop offset=".48" stop-color="#a2b990" stop-opacity=".55"/><stop offset=".72" stop-color="#789e4b" stop-opacity=".3"/><stop offset="1" stop-color="#415543" stop-opacity="0"/></linearGradient><filter id="soft" x="-30%" y="-60%" width="160%" height="220%"><feGaussianBlur stdDeviation="24"/></filter><filter id="edge" x="-30%" y="-60%" width="160%" height="220%"><feGaussianBlur stdDeviation="9"/></filter></defs><g fill="none" stroke="url(#light)"><path d="M-280 880 C100 1100 270 240 660 530 S1240 1070 1880 170" stroke-width="108" filter="url(#soft)"/><path d="M-280 868 C100 1088 270 228 660 518 S1240 1058 1880 158" stroke-width="15" opacity=".45" filter="url(#edge)"/></g></svg>\n'''
    return {'assets/grain.png':grain,'assets/aurora.svg':ribbon}


def render_pages() -> dict[str,str]:
    return {'index.html':welcome_page(),'chat.html':chat_page(),'history.html':history_page(),
            'models.html':models_page(),'settings.html':settings_page(),'usage.html':usage_page(),
            'states.html':states_page()}


def main() -> None:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args()
    pages=render_pages()
    for filename,artifact in {**pages,**render_stars(),**render_atmosphere_assets()}.items():
        data=artifact.encode("utf-8") if isinstance(artifact,str) else artifact
        path=WEB/filename
        if args.check:
            if not path.exists() or path.read_bytes() != data:
                raise SystemExit(f'Generated output is stale: {path.relative_to(ROOT)}')
        else:
            path.write_bytes(data)
    print(f'{"Verified" if args.check else "Rendered"} {len(pages)} deterministic HTML pages.')


if __name__ == '__main__':
    main()
