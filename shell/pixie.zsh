# pixie.zsh — shell integration for FaeOS
# Source from ~/.zshrc:  source ~/faeos/shell/pixie.zsh
# Or after install:      source ~/.config/pixie/pixie.zsh
#
# Does NOT include personal plugins (autosuggestions), grok paths, or secrets.

# ── PATH ──────────────────────────────────────────────────────
export PATH="$HOME/bin:$HOME/.local/bin:$PATH"

# ── SSH agent ──────────────────────────────────────────────────
# One shared agent for every shell: systemd --user owns the socket at
# ~/.ssh/agent.sock (see systemd/ssh-agent.service). Never clobbers an
# agent inherited from `ssh -A`; falls back to a private spawn if the
# unit can't be reached.
_faeos_sock="$HOME/.ssh/agent.sock"
if [[ -n "$SSH_AUTH_SOCK" && -S "$SSH_AUTH_SOCK" ]]; then
  : # keep inherited/forwarded agent
elif [[ -S "$_faeos_sock" ]]; then
  export SSH_AUTH_SOCK="$_faeos_sock"
elif command -v systemctl >/dev/null 2>&1 && \
     systemctl --user start ssh-agent.service >/dev/null 2>&1 && \
     [[ -S "$_faeos_sock" ]]; then
  export SSH_AUTH_SOCK="$_faeos_sock"
elif command -v ssh-agent >/dev/null 2>&1; then
  eval "$(ssh-agent -s)" >/dev/null 2>&1
fi
unset _faeos_sock
if [[ -f "$HOME/.ssh/faeos_push" ]] && command -v ssh-add >/dev/null 2>&1; then
  ssh-add -l >/dev/null 2>&1 || ssh-add "$HOME/.ssh/faeos_push" >/dev/null 2>&1
fi

# ── Palette / colors ──────────────────────────────────────────
[[ -r "$HOME/.config/palette.env" ]] && source "$HOME/.config/palette.env"
unset NO_COLOR
export FORCE_COLOR=1
export CLICOLOR=1
export LS_COLORS="${LS_COLORS:-di=38;2;232;121;160:ln=38;2;240;180;200:ex=38;2;61;214;140}"
export GREP_COLORS="${GREP_COLORS:-ms=1;38;2;232;121;160:mc=1;38;2;232;121;160}"
export FZF_DEFAULT_OPTS="${FZF_DEFAULT_OPTS:---color=fg:#c0c0c8,bg:#1a0a12,hl:#e879a0,fg+:#ffebf2,bg+:#2a1520,hl+:#ff2d55,info:#9d5c75,prompt:#e879a0,pointer:#ffb020,marker:#3dd68c,spinner:#c44d7a,header:#c44d7a}"

alias ll="${aliases[ll]:-ls -lah --color=auto}"
alias ls="${aliases[ls]:-ls --color=auto}"
alias grep="${aliases[grep]:-grep --color=auto}"
# open scroll → command directory (help page) · open spellbook → file manager
# optional: alias mail='aerc'  # if you use aerc

# ── Starship (prompt box) ─────────────────────────────────────
if command -v starship >/dev/null 2>&1; then
  eval "$(starship init zsh)"
fi

# ── Status box ────────────────────────────────────────────────
_pixie_print_box() {
  [[ -n ${PIXIE_NO_BOX:-} ]] && return 0
  if [[ -n ${_SCRY_FD_READY:-} ]]; then
    "$HOME/bin/starship-box" --status >&3 2>/dev/null || "$HOME/bin/starship-box" --status
  else
    "$HOME/bin/starship-box" --status
  fi
}

_precmd_prompt() {
  print
}

# ── Music (Siren) ─────────────────────────────────────────────
# All playback is handled by the `siren` command (mpv over IPC).
#   siren play | next | prev | stop | pause | now
_MPV_SOCK="${SIREN_SOCK:-/tmp/siren-mpv.sock}"

volume() {
  if ! command -v wpctl >/dev/null 2>&1; then
    echo "volume needs wpctl (PipeWire)"
    return 1
  fi
  if [[ -z "$1" ]]; then
    wpctl get-volume @DEFAULT_AUDIO_SINK@
    return
  fi
  if [[ ! "$1" =~ '^[0-9]+$' ]] || (( $1 > 150 )); then
    echo "Usage: volume <0-150>"
    return 1
  fi
  wpctl set-volume @DEFAULT_AUDIO_SINK@ "${1}%"
  wpctl get-volume @DEFAULT_AUDIO_SINK@
}

# optional yt helper (needs yt-dlp + fzf + mpv)
yt() {
  command -v yt-dlp >/dev/null || { echo "need yt-dlp"; return 1; }
  command -v fzf >/dev/null || { echo "need fzf"; return 1; }
  yt-dlp --get-id --get-title "ytsearch10:$*" 2>/dev/null | paste - - | fzf | awk '{print $NF}' | xargs -I{} mpv "ytdl://{}"
}

# ── Tick + grace + up ─────────────────────────────────────────
_TICK_CFG="${XDG_CONFIG_HOME:-$HOME/.config}/pixie/tick"
_TICK_LAST_SONG=""
_TICK_LAST_TS=0
_TICK_POLL=2

_tick_enabled() {
  [[ -f "$_TICK_CFG" ]] || return 0
  ! grep -qiE '^(off|0|false|no)\b' "$_TICK_CFG" 2>/dev/null
}
_tick_seconds() {
  local s=10 line
  if [[ -f "$_TICK_CFG" ]]; then
    line=$(head -1 "$_TICK_CFG" 2>/dev/null)
    if [[ "$line" =~ '^(on|1)?[[:space:]]*([0-9]+)$' ]]; then
      s="${match[2]}"
    elif [[ "$line" =~ '^([0-9]+)$' ]]; then
      s="${match[1]}"
    fi
  fi
  (( s >= 2 && s <= 120 )) || s=10
  print -r -- "$s"
}
_tick_song_id() {
  if [[ ! -S ${_MPV_SOCK:-/dev/null} ]]; then
    print -r -- "idle"
    return
  fi
  local id
  id=$(MPV_SOCK="$_MPV_SOCK" python3 -c '
import json,os,socket
sock=os.environ.get("MPV_SOCK","")
try:
 s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM); s.settimeout(0.2); s.connect(sock)
 s.sendall(b"{\"command\":[\"get_property\",\"path\"]}\n")
 d=b""
 while True:
  c=s.recv(4096)
  if not c: break
  d+=c
  if b"\n" in d: break
 s.close()
 o=json.loads(d.decode().split("\n",1)[0])
 print(o.get("data") or "playing")
except Exception:
 print("playing")
' 2>/dev/null)
  print -r -- "${id:-playing}"
}
_pixie_busy() {
  local d="${XDG_RUNTIME_DIR:-/tmp}" f pid
  local busy=0
  for f in "$d"/pixie-busy.*(N); do
    [[ -e $f ]] || continue
    pid=${f##*.}
    if [[ $pid == <1-> ]] && kill -0 "$pid" 2>/dev/null; then
      busy=1
    else
      rm -f -- "$f" 2>/dev/null
    fi
  done
  (( busy ))
}
_pixie_in_grace() {
  "$HOME/bin/pixie-session" in-grace 2>/dev/null
}
_pixie_end_grace() {
  "$HOME/bin/pixie-session" grace-clear 2>/dev/null || true
}
_pixie_accept_line() {
  _pixie_end_grace
  zle .accept-line
}
zle -N accept-line _pixie_accept_line

_tick_refresh() {
  # Idle tick only: wipe screen and redraw prompt
  clear
  if zle; then
    zle reset-prompt
    zle -R
  fi
  _TICK_LAST_TS=$EPOCHSECONDS
}

# Clear BEFORE commands (preexec), never AFTER (precmd). Idle tick still clears.
_preexec_clear() {
  [[ -n "${1//[$' \t']/}" ]] || return 0
  _pixie_busy && return 0
  clear
  _pixie_print_box
}
_precmd_tick_arm() {
  # Full idle interval to read output before tick wipe
  _TICK_LAST_TS=$EPOCHSECONDS
}
typeset -ga precmd_functions preexec_functions
precmd_functions=(${precmd_functions:#_precmd_clear})
precmd_functions=(_precmd_tick_arm ${precmd_functions:#_precmd_tick_arm})
precmd_functions=(_precmd_prompt ${precmd_functions:#_precmd_prompt})
preexec_functions=(_preexec_clear ${preexec_functions:#_preexec_clear})


tick() {
  local sub="${1:-status}" n="${2:-}"
  mkdir -p "${_TICK_CFG:h}" 2>/dev/null
  case "$sub" in
    on)
      local sec="${n:-10}"
      (( sec >= 2 && sec <= 120 )) || sec=10
      print -r -- "on $sec" >"$_TICK_CFG"
      PERIOD=$_TICK_POLL
      print "tick: on (every ${sec}s + song change while idle)"
      print "  clear: before commands + idle tick (not after output)"
      ;;
    off)
      print -r -- "off" >"$_TICK_CFG"
      PERIOD=0
      print "tick: off"
      ;;
    status)
      if _tick_enabled; then
        print "tick: on — every $(_tick_seconds)s + song change (idle)"
        print "  clear: preexec + idle timer; no post-cmd wipe"
        if _pixie_in_grace; then
          print "  read-grace: $("$HOME/bin/pixie-session" grace-left)s left (up to re-read)"
        fi
      else
        print "tick: off"
      fi
      ;;
    *)
      print "Usage: tick {on [seconds]|off|status}"
      return 1
      ;;
  esac
}
_pixie_fzf_opts() {
  print -r -- "--height=80% --layout=reverse --border=rounded --cycle --info=inline --ansi --color=fg:#c0c0c8,bg:#1a0a12,hl:#e879a0,fg+:#ffebf2,bg+:#2a1520,hl+:#ff2d55,info:#9d5c75,prompt:#e879a0,pointer:#ffb020,marker:#3dd68c,spinner:#c44d7a,header:#c44d7a"
}

# SCRY — Shift-Tab history sight (tick paused). Real TTY only (no pipe).
scry() {
  export HISTFILE="${HISTFILE:-$HOME/.histfile}"
  "$HOME/bin/pixie-session" grace 600 >/dev/null 2>&1 || true
  _TICK_LAST_TS=$EPOCHSECONDS
  local rf ec=0
  rf=$(mktemp "${TMPDIR:-/tmp}/scry-rerun.XXXXXX" 2>/dev/null) || rf=""
  if [[ -n $rf ]]; then
    "$HOME/bin/scry" --rerun-file "$rf" || ec=$?
  else
    "$HOME/bin/scry" || ec=$?
  fi
  if [[ -n $rf && -s $rf ]]; then
    print -z -- "$(<"$rf")"
  fi
  [[ -n $rf ]] && rm -f -- "$rf" 2>/dev/null
  if (( ec != 0 )); then
    "$HOME/bin/pixie-session" grace 15 >/dev/null 2>&1 || true
    return "$ec"
  fi
  "$HOME/bin/pixie-session" grace 20 >/dev/null 2>&1 || true
  return 0
}
_scry-widget() {
  zle -I
  scry
  zle reset-prompt
  zle -R
}
zle -N _scry-widget
bindkey '^[[Z' _scry-widget
bindkey '\e[Z' _scry-widget
bindkey '\e[27;2;9~' _scry-widget

# ── Scroll / Summon — page book + PATH leaf ──
# App runes **run** tools. PATH / summon selection is printed for print -z.
summon() {
  case "${1:-}" in
    -x|--exec|-l|--list|--refresh|-h|--help) command summon "$@"; return $? ;;
  esac
  local selected ec=0
  selected=$(command summon "$@") || ec=$?
  (( ec != 0 )) && return "$ec"
  [[ -z $selected ]] && return 1
  print -z -- "$selected"
}

scroll() {
  case "${1:-}" in
    list|menu|-h|--help|--list|-l|--refresh|--path|-p|path)
      # --path alone still needs print-z when user picks
      if [[ "${1:-}" == --path || "${1:-}" == -p || "${1:-}" == path ]]; then
        local selected ec=0
        selected=$(command scroll "$@") || ec=$?
        (( ec != 0 )) && return "$ec"
        [[ -z $selected ]] && return 1
        print -z -- "$selected"
        return 0
      fi
      command scroll "$@"
      return $?
      ;;
  esac
  # full book: runes exec apps — do not capture stdout
  command scroll "$@"
}

# open scroll → command directory (help page) · open spellbook → file manager
up() {
  local arg="${1:-}"
  export HISTFILE="${HISTFILE:-$HOME/.histfile}"
  _TICK_LAST_TS=$EPOCHSECONDS
  case "$arg" in
    message|msg|last|pixie|reply|events|event|log)
      "$HOME/bin/pixie-session" up "$arg"
      "$HOME/bin/pixie-session" grace >/dev/null 2>&1 || true
      return $?
      ;;
    list|box|plain|history)
      "$HOME/bin/pixie-session" up history
      "$HOME/bin/pixie-session" grace >/dev/null 2>&1 || true
      return $?
      ;;
  esac
  if [[ "$arg" == <-> ]]; then
    "$HOME/bin/pixie-session" up "$arg"
    "$HOME/bin/pixie-session" grace >/dev/null 2>&1 || true
    return $?
  fi
  if ! command -v fzf >/dev/null 2>&1; then
    "$HOME/bin/pixie-session" up history
    "$HOME/bin/pixie-session" grace >/dev/null 2>&1 || true
    return $?
  fi
  local selected
  selected=$(
    fc -rl 1 2>/dev/null \
      | FZF_DEFAULT_OPTS="$(_pixie_fzf_opts)" fzf \
          --header='history  ·  ↑↓ move  ·  type to filter  ·  Enter = put on prompt  ·  Esc = quit' \
          --prompt='up › ' \
          --tiebreak=index \
      | sed -E 's/^[[:space:]]*[0-9]+[[:space:]]+//'
  ) || return $?
  [[ -z "$selected" ]] && return 1
  print -z -- "$selected"
}
periodic() {
  _tick_enabled || return 0
  _pixie_busy && return 0
  _pixie_in_grace && return 0
  local song need_clear=0
  song=$(_tick_song_id)
  if [[ "$song" != "$_TICK_LAST_SONG" ]]; then
    _TICK_LAST_SONG=$song
    need_clear=1
  fi
  local now=$EPOCHSECONDS
  local interval=$(_tick_seconds)
  if (( now - _TICK_LAST_TS >= interval )); then
    need_clear=1
  fi
  (( need_clear )) || return 0
  _pixie_busy && return 0
  _pixie_in_grace && return 0
  _tick_refresh
}
if _tick_enabled; then
  PERIOD=$_TICK_POLL
  _TICK_LAST_TS=$EPOCHSECONDS
  _TICK_LAST_SONG=$(_tick_song_id)
else
  PERIOD=0
fi
export LESS="-F -X"
