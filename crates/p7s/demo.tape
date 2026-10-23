# p7s README demo — canned data only (`p7s --demo`), so no real host,
# user, PID or command is ever recorded.
# Prereq: cargo build --release -p p7s

Output demo.gif

Set Shell "bash"
Set Width 1400
Set Height 900
Set Theme "Catppuccin Mocha"
Set FontSize 16
Set Padding 24
Set TypingSpeed 35ms
Set Framerate 24

Hide
Type "PS1='p7s $ '"
Enter
Type "clear"
Enter
Type "./demo/run.sh"
Enter
Sleep 2s
Show

# process list, sorted by CPU by default
Sleep 1.5s

# sort by memory
Type "m"
Sleep 1.5s

# filter
Type "/"
Type "nginx"
Sleep 1.5s
Enter
Sleep 1s
Type "q"
Sleep 800ms

# describe the selected process
Type "d"
Sleep 2s
Escape
Sleep 800ms

# system summary
Type "i"
Sleep 2s
Escape
Sleep 800ms

# signal picker (demo never signals anything real)
Type "K"
Sleep 2s
Escape
Sleep 800ms

Type "q"
Sleep 800ms
