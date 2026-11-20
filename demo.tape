# wrkflw demo tape
#
# Render with: vhs demo.tape   (from the repository root, after
# `cargo build --release`; needs Docker running)
#
# Shows the headline: `wrkflw doctor`, a run that catches a failing test
# with its failure digest, the fix, a pass that says what it couldn't
# check (a macos-latest job run as Linux), and `--macos-on-host` turning
# that into a verified pass. examples/gate-demo/greet.sh is put back as it
# was at the end, so the tape can be recorded again.

Output demo.gif

Set Shell "zsh"
Set Theme "Catppuccin Mocha"
Set FontSize 18
Set Width 1680
Set Height 1000
Set Padding 30
Set TypingSpeed 45ms
Set PlaybackSpeed 1.0

Hide
Type `export PATH=$PWD/target/release:$PATH && cd examples/gate-demo && sed -i '' 's/Hello, $1!"/Hello, $1"/' greet.sh && clear`
Enter
Sleep 500ms
Show

# ── What a run here would use ────────────────────────────────────
Type "wrkflw doctor"
Sleep 300ms
Enter
Wait+Screen@60s /workflows /
Sleep 6s

Type "clear"
Enter

# ── A run that catches a failing test ────────────────────────────
Type "wrkflw run .github/workflows/ci.yml"
Sleep 300ms
Enter
Wait+Screen@90s /Rerun: wrkflw runs replay/
Sleep 7s

Type "clear"
Enter

# ── Fix it ───────────────────────────────────────────────────────
Type `sed -i '' 's/Hello, $1"/Hello, $1!"/' greet.sh && cat greet.sh`
Sleep 300ms
Enter
Sleep 2s

# ── A pass that says what it couldn't check ──────────────────────
Type "wrkflw run .github/workflows/ci.yml"
Sleep 300ms
Enter
Wait+Screen@90s /Job succeeded: package/
Sleep 7s

Type "clear"
Enter

# ── Run the macOS job on this Mac: a verified pass ───────────────
Type "wrkflw run --macos-on-host .github/workflows/ci.yml"
Sleep 300ms
Enter
Wait+Screen@90s /Job succeeded: package/
Sleep 7s

Hide
Type `sed -i '' 's/Hello, $1!"/Hello, $1"/' greet.sh`
Enter
Show
