# Frame scheduling and presentation

Polaris uses a demand-driven 60 fps ceiling, not a continuously running animation loop. Raw terminal records are decoded before requesting a frame. Partial escape sequences, key releases and unchanged hover targets do not invalidate the screen. Ordered keys, paste, clicks, drag boundaries and wheel events are preserved; consecutive ordinary mouse moves and resize notifications are coalesced. Input batches are bounded by four milliseconds or 256 semantic events. A pending escape is not expired while further native input records are queued.

## One frame and shared layouts

Frame preparation captures one monotonic time and terminal size. Timer text uses that captured time throughout the frame. Working, goal and agent elapsed labels invalidate at displayed second boundaries; simulation ticks remain independent. Clipboard and file-index work stays asynchronous. Hidden completion-index updates do not repaint Home.

One active transcript/width/theme generation caches each entry's styled lines, exact wrap counts, source descriptors and token decorations. Unchanged entries survive appends and individual tool expansion. A cached history pane reuses its painted cells and mouse/source map during composer-only editing. Hover and selection changes invalidate the pane without rebuilding unchanged entry layouts. Only visible source descriptors gain display-cell mappings; complete semantic source remains available for cross-message copying. Editor sizing and painting also share wrapped rows. Width/theme/conversation changes release obsolete derived generations. Cold reflow still costs work proportional to transcript size.

Mouse dispatch refreshes stale geometry in memory with the same painter and cached layouts; it does not write a pre-click terminal frame. This preserves typing-then-click ordering, wrapped Unicode offsets, attachment expansion and word selection. TextRegion.source now uses Arc<str> so cloned maps share immutable source text; copy_range still returns a String.

## Terminal output and cursor

The backend records cursor intent and skips empty cell diffs. A FrameOutput writer buffers Crossterm commands, including internal flush requests. Presentation commits the complete command buffer with one writer transaction and final flush. Unchanged cells plus unchanged cursor/visibility produce zero bytes. Cursor-only movement emits a positioning command without a hide/reveal pair.

A nonblocking DEC mode 2026 query enables synchronized output when the terminal reports support. The commit then wraps the changed frame and final cursor position in balanced Begin/End Synchronized Update commands. Without support or a reply, the buffered fallback hides during painting, positions the cursor and reveals it. Partial-write and flush failures attempt to end synchronization and reveal the cursor before session cleanup.

Cursor blinking is disabled for the session. A DEC mode 12 reply records the original blink state and retains the current shape. If no supported reply arrives within 100 ms, the next loop uses a steady block. Exit restores the reported blink state, or the terminal's configured default cursor style for the fallback. Capability responses are decoded separately from editing, including split and late responses; reply-shaped text inside bracketed paste remains literal text. Session guard cleanup also covers ordinary failures and unwinding panics.

## Evidence and limits

The implementation is a UI preview without agent/backend execution. Rendering tests distinguish CPU layout time, emitted application bytes and the terminal emulator's own presentation. A buffer benchmark does not measure physical display latency. OS input transport can impose its own throughput limits; instantaneous decoder bursts and sustained ConPTY input are checked separately.

Implementation evidence and current package checks: [verification](rendering-verification.md).
