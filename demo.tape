# wrkflw TUI demo tape
#
# Render with: vhs demo.tape
#
# Showcases the redesigned TUI: Workflows list, Tweaks overlay with
# accent cycling, DAG graph across different workflow shapes, Trigger,
# Secrets, Help tabs, and a live run with Execution + Logs streaming.

Output demo.gif

Set Shell "zsh"
Set Theme "Catppuccin Mocha"
Set FontSize 14
Set Width 1280
Set Height 760
Set Padding 20
Set TypingSpeed 55ms
Set PlaybackSpeed 1.0

# ── Launch ────────────────────────────────────────────────────────
Hide
Type "export PATH=target/release:$PATH"
Enter
Type "clear"
Enter
Show

Sleep 500ms
Type "wrkflw tui examples/ui-demo -r podman"
Sleep 500ms
Enter

Sleep 2500ms

# ── Workflows tab: browse cards ──────────────────────────────────
Down@320ms 5
Sleep 700ms
Up@320ms 3
Sleep 800ms

# ── Tweaks overlay: cycle accent colors ──────────────────────────
Type ","
Sleep 1200ms
Type "a"
Sleep 700ms
Type "a"
Sleep 700ms
Type "a"
Sleep 700ms
Type "a"
Sleep 800ms
Escape
Sleep 700ms

# ── DAG tab: diamond shape (workflow 01) ─────────────────────────
Up@250ms 10
Sleep 300ms
Type "3"
Sleep 2200ms

# ── DAG tab: wide fan-out (workflow 02) ──────────────────────────
Type "1"
Sleep 300ms
Down@300ms 1
Sleep 300ms
Type "3"
Sleep 2200ms

# ── DAG tab: linear chain (workflow 03) ──────────────────────────
Type "1"
Sleep 300ms
Down@300ms 1
Sleep 300ms
Type "3"
Sleep 2000ms

# ── Trigger tab ──────────────────────────────────────────────────
Type "5"
Sleep 2200ms

# ── Secrets tab ──────────────────────────────────────────────────
Type "6"
Sleep 2200ms

# ── Help overlay ─────────────────────────────────────────────────
Type "7"
Sleep 2000ms
Down@150ms 6
Sleep 900ms

# ── Run: select linear workflow and kick it off ──────────────────
Type "1"
Sleep 400ms
Space
Sleep 600ms
Type "r"
Sleep 1200ms

# ── Execution tab: watch the run ─────────────────────────────────
Type "2"
Sleep 5000ms

# ── Logs tab: streaming output ───────────────────────────────────
Type "4"
Sleep 2500ms
Down@120ms 15
Sleep 1500ms

# ── Quit (q, confirm q) ──────────────────────────────────────────
Type "q"
Sleep 500ms
Type "q"
Sleep 300ms
