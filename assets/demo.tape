# galdr demo — regenerate with:  vhs assets/demo.tape
# Runs in an isolated demo HOME so it never touches your real ~/.galdr. The setup block
# (seeding recordings by piping events to the sensor — what your harness does for real on
# every PostToolUse) is wiped with `clear`; the shown part is only real galdr commands.

Output assets/demo.gif

Set Shell "bash"
Set FontSize 16
Set Width 1180
Set Height 720
Set Padding 22
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms

# --- setup (wiped by `clear`): isolated HOME, one task recorded, one shape recorded twice ---
Type `export PATH="$HOME/.cargo/bin:$PATH"; export HOME=/tmp/galdr-demo-vhs; rm -rf "$HOME"; mkdir -p "$HOME"` Enter
Type `tool(){ printf '{"tool_name":"%s","tool_input":%s,"tool_response":{}}\n' "$1" "$2" | galdr hook >/dev/null; }` Enter
Type `galdr rec start ship-preview >/dev/null; tool Bash '{"command":"git status"}'; tool Bash '{"command":"bun run build"}'; tool Write '{"file_path":"/repo/preview-url.txt"}'; galdr rec stop >/dev/null` Enter
Type `SHIP=$(ls -t "$HOME/.galdr/recordings"/*.json | head -1 | xargs basename | sed 's/\.json//')` Enter
Type `for i in 1 2; do galdr rec start weekly-report >/dev/null; tool Bash '{"command":"git log --oneline -20"}'; tool Read '{"file_path":"/repo/CHANGELOG.md"}'; tool Write '{"file_path":"/repo/weekly.md"}'; galdr rec stop >/dev/null; done` Enter
Type `clear` Enter
Sleep 600ms

# --- the demo ---
Type "#  teach your agent a task once — replay it as a skill" Enter
Sleep 900ms
Type "galdr list" Enter
Sleep 1.9s

Type "#  galdr captured tool calls, not pixels:" Enter
Sleep 500ms
Type "galdr show $SHIP" Enter
Sleep 2.2s

Type "galdr distill $SHIP" Enter
Sleep 2.4s

Type "#  later, you record how the replay went:" Enter
Sleep 500ms
Type "galdr outcome usage --skill galdr-ship-preview --rec $SHIP --outcome success" Enter
Sleep 1.6s

Type "#  which repeated tasks are worth a skill?" Enter
Sleep 500ms
Type "galdr suggest" Enter
Sleep 2.3s

Type "#  how reliably do my skills replay?" Enter
Sleep 500ms
Type "galdr bench" Enter
Sleep 2.8s
