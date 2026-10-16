# Commands follow the host card

Read the host card in this prompt. Use that package manager. Do not guess Ubuntu.

- pacman box: install with `alchemy brew <pkg>` (or `pacman -S` only if the human asked for pacman). Update: `alchemy sip`. Never `apt`, `apt-get`, `yum`, `dnf`, `apk`.
- apt-get box: Debian/Ubuntu family. Never pacman.
- dnf box: Fedora family. Never pacman or apt.
- apk box: Alpine. Never apt or pacman.
- brew box: macOS/homebrew.

Shell is argv-only. Paths: `~/bin` is on PATH. This is systemd if the host card says so.
