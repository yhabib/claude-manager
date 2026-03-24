"""Fake Claude Code selection prompt for integration testing.

Presents a selection UI that responds to arrow keys and Enter,
just like Claude Code's permission prompts. Writes the selected
option number to a result file.
"""
import sys
import tty
import termios

OPTIONS = ["1. Yes", "2. Yes, and don't ask again", "3. No"]
RESULT_FILE = sys.argv[1] if len(sys.argv) > 1 else "/tmp/claude_manager_test_result"

def read_key():
    fd = sys.stdin.fileno()
    old = termios.tcgetattr(fd)
    try:
        tty.setraw(fd)
        ch = sys.stdin.read(1)
        if ch == '\x1b':
            ch2 = sys.stdin.read(1)
            ch3 = sys.stdin.read(1)
            if ch2 == '[':
                if ch3 == 'A': return 'up'
                if ch3 == 'B': return 'down'
        if ch in ('\r', '\n'):
            return 'enter'
        return ch
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, old)

def render(selected):
    sys.stdout.write('\r\033[K')
    for i, opt in enumerate(OPTIONS):
        prefix = '> ' if i == selected else '  '
        sys.stdout.write(f'\r\033[K{prefix}{opt}\n')
    # Move cursor back up
    sys.stdout.write(f'\033[{len(OPTIONS)}A')
    sys.stdout.flush()

selected = 0
render(selected)

while True:
    key = read_key()
    if key == 'down':
        selected = min(selected + 1, len(OPTIONS) - 1)
        render(selected)
    elif key == 'up':
        selected = max(selected - 1, 0)
        render(selected)
    elif key == 'enter':
        # Move past the options
        sys.stdout.write('\n' * len(OPTIONS))
        sys.stdout.write(f'\r\033[KSelected option: {selected + 1}\n')
        sys.stdout.flush()
        with open(RESULT_FILE, 'w') as f:
            f.write(str(selected + 1))
        break
