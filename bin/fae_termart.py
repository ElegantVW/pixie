#!/usr/bin/env python3
"""Shared terminal art for the Pixie stack — pink, whimsical, consistent boxes.

Used by scry, ask/pixie, cmds, duck, etc. Import from ~/bin:

    import sys
    from pathlib import Path
    sys.path.insert(0, str(Path.home() / "bin"))
    import fae_termart as art
"""
from __future__ import annotations

import atexit
import fcntl
import os
import re
import select
import shutil
import struct
import subprocess
import sys
import termios
import time
import unicodedata
from dataclasses import dataclass
from typing import Iterable

ANSI_RE = re.compile(r"\033\[[0-9;]*[mK]")

# Optional controlling TTY fd for size + live redraw (set by scry/siren TUIs).
_size_fd: int | None = None
# Force color even when stdout is not a tty (drawing goes to /dev/tty).
_force_color: bool = False


class Palette:
    RESET = "\033[0m"
    BOLD = "\033[1m"
    ITALIC = "\033[3m"
    DIM = "\033[2m"
    PINK = "\033[38;5;175m"
    DARK = "\033[38;5;168m"
    PINK_DIM = "\033[38;5;132m"
    BLUSH = "\033[38;5;218m"
    SILVER = "\033[38;5;252m"
    MUTED = "\033[38;5;245m"
    OK = "\033[38;5;78m"
    WARN = "\033[38;5;214m"
    ERR = "\033[38;5;197m"


P = Palette

# Shared wait messages for when Pixie is processing
PIXIE_WAIT = (
    "Pixie is poking around…",
    "Contacting the fey world, please wait…",
    "Pixie is dusting off her spellbook…",
    "Pixie is chasing a thought through the wires…",
    "Pixie is listening to the machine spirits…",
    "Pixie is braiding a reply out of moonlight…",
    "Hold still — Pixie is concentrating…",
    "Pixie wandered into /tmp and got distracted…",
    "Pixie is counting sparkles in the cache…",
    "Pixie is negotiating with the kernel gnomes…",
    "Pixie is untangling a knot of pointers…",
    "Pixie is whispering to the inode fairies…",
    "Pixie is polishing her answer with starlight…",
    "Pixie is peeking under the root of the problem…",
    "Pixie is sipping tea from a thimble of RAM…",
    "Pixie is mapping the lay of the home directory…",
    "Pixie is asking the pipes which way is north…",
    "Pixie is humming a little compile song…",
    "Pixie is sorting wishes by priority…",
    "Pixie is chasing a runaway null…",
    "Pixie is rolling up her sleeves (figuratively)…",
    "Pixie is consulting the ancient man pages…",
    "Pixie is balancing on a stack frame…",
    "Pixie is bribing the scheduler with cookies…",
    "Pixie is drawing a tiny map of your request…",
    "Pixie is waiting for the moon to align with /dev…",
    "Pixie is shaking the magic 8-bit ball…",
    "Pixie is retying her boots for another try…",
    "Pixie is listening for the click of a good idea…",
    "Pixie is sweeping cobwebs out of the heap…",
)

# Shared paths
from pathlib import Path
BIN = Path.home() / "bin"
PIXIE_SCREEN = BIN / "pixie-screen"


def bind_tty(fd: int | None, *, color: bool = True) -> None:
    """Bind ioctl size + color to a controlling TTY for live TUIs."""
    global _size_fd, _force_color
    _size_fd = fd
    _force_color = bool(color and fd is not None)


def color_ok() -> bool:
    if os.environ.get("NO_COLOR", "").strip():
        return False
    if _force_color:
        return True
    if os.environ.get("FORCE_COLOR", "").strip() or os.environ.get("CLICOLOR_FORCE", "").strip():
        return True
    try:
        return os.isatty(1) or os.isatty(2)
    except Exception:
        return False


def paint(text: str, *codes: str) -> str:
    if not color_ok() or not codes:
        return text
    return "".join(codes) + text + P.RESET


def fae_error(creature: str, msg: str, next: str | None = None, *, detail: str | None = None) -> None:
    """House-wide stderr voice — see docs/error-voice.md.

    Prints: ``<creature>: <msg>`` plus optional ``next:`` / ``detail:``
    (detail only when FAE_DEBUG is set, or when *detail* is forced by caller
    after checking the env themselves).
    """
    import os
    import sys

    name = (creature or "fae").strip().lower() or "fae"
    line = paint(f"{name}: {msg}", P.ERR) if color_ok() else f"{name}: {msg}"
    print(line, file=sys.stderr)
    if next:
        print(f"  next:  {next}", file=sys.stderr)
    dbg = os.environ.get("FAE_DEBUG", "").strip() not in ("", "0", "false", "no")
    if dbg and detail:
        print(f"  detail: {detail}", file=sys.stderr)


def strip_ansi(s: str) -> str:
    return ANSI_RE.sub("", s)


def _char_width(ch: str) -> int:
    """Column width for monospace terminals (kmscon/xterm).

    Box-drawing and dingbats (✦ ♪) are width 1 here — treating them as 2
    misaligns right borders on most European fonts.
    """
    if not ch:
        return 0
    o = ord(ch)
    if o < 32 or 0x7F <= o < 0xA0:
        return 0
    if unicodedata.combining(ch):
        return 0
    eaw = unicodedata.east_asian_width(ch)
    if eaw in ("F", "W"):
        return 2
    # Emoji & symbols that are double-width on most terminals
    if 0x1F300 <= o <= 0x1FAFF:
        return 2
    return 1


def vis_len(s: str) -> int:
    return sum(_char_width(c) for c in strip_ansi(s))


def winsize(fd: int | None = None, default: tuple[int, int] = (80, 24)) -> tuple[int, int]:
    """Return (cols, rows) via TIOCGWINSZ on fd, else shutil / defaults."""
    for candidate in (fd, _size_fd, 1, 0):
        if candidate is None:
            continue
        try:
            raw = fcntl.ioctl(candidate, termios.TIOCGWINSZ, b"\x00" * 8)
            rows, cols, _, _ = struct.unpack("HHHH", raw)
            if cols > 0 and rows > 0:
                return int(cols), int(rows)
        except Exception:
            continue
    try:
        sz = shutil.get_terminal_size(default)
        return int(sz.columns), int(sz.lines)
    except Exception:
        return default


def term_width(default: int = 72, *, cap: int | None = 96, floor: int = 44, margin: int = 1) -> int:
    """Usable box width. margin keeps the last column free so print+newline
    does not auto-wrap (kmscon/xterm xenl) and shatter right borders on n/p.
    cap=None: no upper clamp (fullscreen TUIs). Default cap 96 for one-shot boxes."""
    cols, _ = winsize(_size_fd)
    if cols <= 0:
        cols = default
    usable = max(1, cols - max(0, margin))
    if cap is None:
        return max(floor, usable)
    return max(floor, min(usable, cap))


def term_height(default: int = 24) -> int:
    _, rows = winsize(_size_fd)
    if rows <= 0:
        rows = default
    return max(12, rows)


def tty_write(fd: int, text: str) -> None:
    """Write UTF-8 text to a TTY fd (no Python stream buffering surprises).

    If OPOST is off (classic tty.setraw), bare \\n does not return the cursor
    to column 0 — frames staircase. Force CR-LF only in that case. When OPOST
    is on (set_ui_mode / cbreak), leave \\n alone so the driver can map it.
    """
    if text and "\n" in text:
        need_cr = True
        try:
            oflag = termios.tcgetattr(fd)[1]
            # OPOST on → kernel may already expand \\n to CR-LF (ONLCR)
            need_cr = not bool(oflag & termios.OPOST)
        except (termios.error, OSError):
            need_cr = True
        if need_cr:
            text = text.replace("\r\n", "\n").replace("\r", "\n").replace("\n", "\r\n")
    data = text.encode("utf-8", errors="replace")
    view = memoryview(data)
    while view:
        n = os.write(fd, view)
        if n <= 0:
            break
        view = view[n:]


def set_ui_mode(fd: int) -> list:
    """Char-at-a-time input WITHOUT killing output post-processing.

    Prefer this over tty.setraw(): setraw clears OPOST so \\n stops doing
    carriage-return and every redraw after the first looks staircase-broken.
    Returns the previous termios attrs (pass to termios.tcsetattr to restore).
    """
    old = termios.tcgetattr(fd)
    # cbreak: no canonical, no echo; OPOST left alone when possible
    try:
        import tty as _tty

        _tty.setcbreak(fd)
    except Exception:
        # manual fallback: like cbreak but keep ONLCR if present
        new = termios.tcgetattr(fd)
        # iflag
        new[0] &= ~(termios.IGNBRK | termios.BRKINT | termios.PARMRK
                    | termios.ISTRIP | termios.INLCR | termios.IGNCR
                    | termios.ICRNL | termios.IXON)
        # oflag — keep OPOST|ONLCR so newlines still return to col 0
        new[1] |= termios.OPOST
        if hasattr(termios, "ONLCR"):
            new[1] |= termios.ONLCR
        # cflag
        new[2] &= ~termios.CSIZE
        new[2] |= termios.CS8
        # lflag
        new[3] &= ~(termios.ECHO | termios.ECHONL | termios.ICANON
                    | termios.ISIG | termios.IEXTEN)
        new[6][termios.VMIN] = 1
        new[6][termios.VTIME] = 0
        termios.tcsetattr(fd, termios.TCSADRAIN, new)
    # After cbreak, re-assert OPOST|ONLCR and kill IXON so Ctrl-S/Q reach
    # the app (otherwise the line discipline eats them as XOFF/XON).
    try:
        cur = termios.tcgetattr(fd)
        cur[0] &= ~termios.IXON
        cur[1] |= termios.OPOST
        if hasattr(termios, "ONLCR"):
            cur[1] |= termios.ONLCR
        # keep ISIG off in UI mode so Ctrl-C is a key, not a signal
        cur[3] &= ~(termios.ISIG | termios.ICANON | termios.ECHO)
        cur[6][termios.VMIN] = 1
        cur[6][termios.VTIME] = 0
        termios.tcsetattr(fd, termios.TCSADRAIN, cur)
    except (termios.error, OSError):
        pass
    return old


def clear_screen(fd: int | None = None) -> str:
    """Home + erase display. Returns CSI string if no fd."""
    # Avoid \033[3J — not all consoles implement scrollback erase cleanly.
    seq = "\033[H\033[2J"
    if fd is not None:
        # clear seq has no newlines; write raw bytes without CR munging
        data = seq.encode("ascii")
        view = memoryview(data)
        while view:
            n = os.write(fd, view)
            if n <= 0:
                break
            view = view[n:]
        return ""
    return seq


def paint_frame(fd: int, body: str, *, disable_wrap: bool = True) -> None:
    """Atomic full-frame redraw on controlling TTY.

    Cursor home + erase, then body with CR-LF line ends (raw-mode safe).
    """
    # CSI-only prefix (no newlines) written raw so we do not inject CRs mid-escape
    prefix = "\033[H\033[2J"
    if disable_wrap:
        prefix += "\033[?7l"  # DECAWM off — prevent accidental wrap shredding borders
    data = prefix.encode("ascii")
    view = memoryview(data)
    while view:
        n = os.write(fd, view)
        if n <= 0:
            break
        view = view[n:]
    # body via tty_write → CR-LF; skip the trailing newline if the frame fills
    # the screen so we never scroll a full-height frame (which pushes the top
    # border off and shifts everything up one row)
    if not body.endswith("\n"):
        lines = body.count("\n") + 1
        if lines < term_height():
            body = body + "\n"
    body = body + "\033[0m"
    if disable_wrap:
        body = body + "\033[?7h"
    tty_write(fd, body)


def pad_vis(s: str, width: int) -> str:
    n = vis_len(s)
    if n == width:
        return s
    if n < width:
        return s + (" " * (width - n))
    # truncate by visible width — ASCII ellipsis only
    plain = strip_ansi(s)
    out = []
    w = 0
    # reserve 3 cols for "..."
    limit = max(1, width - 3)
    for ch in plain:
        cw = _char_width(ch)
        if w + cw > limit:
            break
        out.append(ch)
        w += cw
    cut = "".join(out) + "..."
    return cut + (" " * max(0, width - vis_len(cut)))


def wrap_plain(text: str, width: int) -> list[str]:
    """Word-wrap a plain (no ANSI) string; preserves empty lines."""
    if width < 8:
        width = 8
    lines: list[str] = []
    for raw in (text.splitlines() or [""]):
        if not raw.strip():
            lines.append("")
            continue
        indent_m = re.match(r"^(\s*)", raw)
        indent = indent_m.group(1) if indent_m else ""
        content = raw[len(indent) :]
        max_c = max(8, width - len(indent))
        if vis_len(raw) <= width:
            lines.append(raw)
            continue
        words = content.split(" ")
        cur = ""
        for word in words:
            trial = (cur + " " + word).strip() if cur else word
            if vis_len(trial) <= max_c:
                cur = trial
            else:
                if cur:
                    lines.append(indent + cur)
                # hard-break overlong tokens
                while vis_len(word) > max_c:
                    chunk = []
                    w = 0
                    for ch in word:
                        cw = _char_width(ch)
                        if w + cw > max_c:
                            break
                        chunk.append(ch)
                        w += cw
                    lines.append(indent + "".join(chunk))
                    word = word[len("".join(chunk)) :]
                cur = word
        if cur:
            lines.append(indent + cur)
    return lines or [""]


def _ascii_box_enabled() -> bool:
    """Default ASCII frames (kmscon-safe). Set PIXIE_UNICODE=1 for rounded art."""
    v = os.environ.get("PIXIE_UNICODE", "").strip().lower()
    if v in ("1", "on", "true", "yes", "unicode"):
        return False
    return True


def truncate_vis(text: str, width: int, *, ellipsis: str | None = None) -> str:
    """Truncate plain or ANSI text to visible width, appending ellipsis if needed."""
    if width <= 0:
        return ""
    plain = strip_ansi(text)
    if vis_len(plain) <= width:
        return plain
    ascii_mode = _ascii_box_enabled()
    ell = ellipsis if ellipsis is not None else ("..." if ascii_mode else "…")
    ell_w = vis_len(ell)
    if width <= ell_w:
        return ell[:width]
    keep = width - ell_w
    # walk codepoints by display width
    out = []
    w = 0
    for ch in plain:
        cw = _char_width(ch)
        if w + cw > keep:
            break
        out.append(ch)
        w += cw
    return "".join(out) + ell


def focus_mark(selected: bool, *, focused_panel: bool = True) -> str:
    """Safe selection prefix (never slice into ANSI). Pink crown when selected."""
    if selected and focused_panel:
        return paint("► ", P.BOLD, P.PINK)
    if selected:
        return paint("• ", P.PINK)
    return "  "


def box(
    body: str | Iterable[str],
    *,
    title: str = "",
    subtitle: str = "",
    accent: str | None = None,
    width: int | None = None,
    body_style: tuple[str, ...] = (),
    cap: int | None = 96,
    floor: int = 36,
) -> str:
    """Frame for panels. Default is pure ASCII (kmscon-safe):

        +-- * Title --------------------+
        | body line                     |
        +-------------------------------+

    Set PIXIE_UNICODE=1 for rounded Unicode borders.
    width is the outer frame width. cap/floor clamp unless cap is None
    (exact width, for side-by-side panels and full-bleed TUI chrome).
    """
    acc = accent if accent is not None else P.PINK_DIM
    w = width or term_width()
    if cap is None:
        outer = max(floor if floor > 0 else 1, w)
    else:
        outer = max(floor, min(w, cap))
    # never exceed requested width when caller passed an explicit width
    if width is not None:
        outer = min(outer, max(1, width)) if cap is not None else max(1, width)
    outer = max(8, outer)
    inner = outer - 2  # between left and right border chars
    body_w = max(1, inner - 2)  # between | sp ... sp |

    ascii_mode = _ascii_box_enabled()
    if ascii_mode:
        tl, tr, bl, br, hz, vt = "+", "+", "+", "+", "-", "|"
        mark = "*"
        ell = "..."
        soft = "-"
    else:
        tl, tr, bl, br, hz, vt = "╭", "╮", "╰", "╯", "─", "│"
        mark = "✦"
        ell = "…"
        soft = "·"

    lines_in: list[str]
    if isinstance(body, str):
        lines_in = []
        for para in body.splitlines() or [""]:
            lines_in.extend(wrap_plain(strip_ansi(para), body_w))
    else:
        lines_in = []
        for para in body:
            for ln in str(para).splitlines() or [""]:
                if ln and strip_ansi(ln) != ln:
                    # pre-styled line: keep ANSI (caller already wrapped it);
                    # too wide → fall back to plain so it gets re-wrapped
                    if vis_len(strip_ansi(ln)) > body_w:
                        ln = strip_ansi(ln)
                    lines_in.append(ln)
                else:
                    lines_in.extend(wrap_plain(ln, body_w))

    # Title bar:  +-- * Title * -----+   (tl + hz + label + fill + tr)
    # Visible: 1 + 1 + len(label) + fill + 1  == outer  => fill = outer - 3 - len(label)
    # With outer = inner + 2: fill = inner - 1 - len(label)
    if title:
        tplain = strip_ansi(title)
        label = f" {mark} {tplain} {mark} "
        if vis_len(label) > inner - 2:
            # keep mark + truncated title
            keep = max(1, inner - 8)
            label = f" {mark} {tplain[:keep]}{ell} {mark} "
            if vis_len(label) > inner - 2:
                label = f" {tplain[: max(1, inner - 4)]} "
        fill = max(0, inner - vis_len(label) - 1)
        top = (
            paint(tl + hz, acc)
            + paint(label, P.BOLD, P.PINK)
            + paint(hz * fill + tr, acc)
        )
    else:
        top = paint(tl + hz * inner + tr, acc)

    out = [top]

    if subtitle:
        for ln in wrap_plain(strip_ansi(subtitle), body_w):
            out.append(
                paint(vt, acc)
                + " "
                + paint(pad_vis(ln, body_w), P.ITALIC, P.DARK)
                + " "
                + paint(vt, acc)
            )
        out.append(
            paint(vt, acc)
            + paint(" " + soft * body_w + " ", acc)
            + paint(vt, acc)
        )

    styles = body_style or (P.SILVER,)
    for ln in lines_in:
        if strip_ansi(ln) != ln:
            # pre-styled line — keep its colors, pad with spaces
            out.append(paint(vt, acc) + " " + ln + " " * max(0, body_w - vis_len(strip_ansi(ln))) + " " + paint(vt, acc))
            continue
        plain = pad_vis(strip_ansi(ln), body_w)
        if plain.strip():
            painted = paint(plain, *styles)
        else:
            painted = " " * body_w
        out.append(paint(vt, acc) + " " + painted + " " + paint(vt, acc))

    out.append(paint(bl + hz * inner + br, acc))
    return "\n".join(out)


def hrule(width: int, title: str = "", *, accent: str | None = None) -> str:
    """Full-width box divider: ├─ title ─┤ (ASCII +-- title --+). vis_len == width."""
    acc = accent if accent is not None else P.PINK_DIM
    w = max(8, int(width))
    inner = w - 2
    if _ascii_box_enabled():
        lt, rt, hz, mark, ell = "+", "+", "-", "*", "..."
    else:
        lt, rt, hz, mark, ell = "├", "┤", "─", "✦", "…"
    if title:
        tplain = strip_ansi(title)
        label = f" {mark} {tplain} {mark} "
        if vis_len(label) > inner - 2:
            keep = max(1, inner - 8)
            label = f" {mark} {tplain[:keep]}{ell} {mark} "
        fill = max(0, inner - vis_len(label) - 1)
        return paint(lt + hz, acc) + paint(label, P.BOLD, P.PINK) + paint(hz * fill + rt, acc)
    return paint(lt + hz * inner + rt, acc)


def rule(width: int | None = None, *, char: str | None = None) -> str:
    w = width or term_width()
    ch = char if char is not None else ("-" if _ascii_box_enabled() else "─")
    return paint(ch * max(8, w), P.PINK_DIM)


def banner(title: str, tagline: str = "", *, width: int | None = None) -> str:
    """Compact title banner (header strip)."""
    return box("", title=title, subtitle=tagline, width=width, body_style=(P.MUTED,))


def panel_lines(
    rows: list[str],
    *,
    title: str = "",
    accent: str | None = None,
    width: int | None = None,
) -> str:
    return box(rows, title=title, accent=accent, width=width)


def footer_keys(pairs: list[tuple[str, str]], *, width: int | None = None) -> str:
    """Key legend in a slim box:  up/dn next  ·  q leave"""
    bits = []
    for key, label in pairs:
        bits.append(f"{key} {label}")
    line = "  |  ".join(bits)
    return box(line, title="runes", width=width, body_style=(P.MUTED,))


def panel(
    body: str | Iterable[str],
    *,
    title: str = "",
    width: int,
    height: int | None = None,
    focus: bool = False,
    accent: str | None = None,
    body_style: tuple[str, ...] = (),
) -> str:
    """Exact-width crystal panel for multi-column TUIs (no 36–96 clamp).

    height, if set, is the outer line count (top + body rows + bottom);
    body is padded or truncated to fit. focus brightens the title accent.
    """
    acc = accent
    if acc is None:
        acc = P.PINK if focus else P.PINK_DIM
    title_s = title
    if focus and title and not title.startswith("►"):
        title_s = f"► {title}"
    # floor=1 + cap=None → honor exact width
    framed = box(
        body,
        title=title_s,
        accent=acc,
        width=width,
        body_style=body_style or (P.SILVER,),
        cap=None,
        floor=1,
    )
    if height is None:
        return framed
    lines = framed.splitlines()
    # box always has top + bottom; pad/truncate middle
    if len(lines) < 2:
        return framed
    top, bot = lines[0], lines[-1]
    mid = lines[1:-1]
    need_mid = max(0, height - 2)
    # empty body row matching border style
    ascii_mode = _ascii_box_enabled()
    vt = "|" if ascii_mode else "│"
    acc_c = acc
    inner = max(1, width - 2)
    body_w = max(1, inner - 2)
    blank = paint(vt, acc_c) + " " + (" " * body_w) + " " + paint(vt, acc_c)
    if len(mid) > need_mid:
        mid = mid[:need_mid]
    else:
        mid = mid + [blank] * (need_mid - len(mid))
    # re-fit top/bot if width drifted (shouldn't)
    return "\n".join([top, *mid, bot])


def split_row(
    left: str,
    right: str,
    *,
    gap: int = 2,
    left_width: int | None = None,
    right_width: int | None = None,
) -> str:
    """Place two multi-line blocks side by side (pad shorter with spaces)."""
    left_lines = left.splitlines() or [""]
    right_lines = right.splitlines() or [""]
    lw = left_width if left_width is not None else max(
        (vis_len(strip_ansi(ln)) for ln in left_lines), default=0
    )
    rw = right_width if right_width is not None else max(
        (vis_len(strip_ansi(ln)) for ln in right_lines), default=0
    )
    gap_s = " " * max(0, gap)
    n = max(len(left_lines), len(right_lines))
    out: list[str] = []
    for i in range(n):
        l = left_lines[i] if i < len(left_lines) else ""
        r = right_lines[i] if i < len(right_lines) else ""
        lpad = l + " " * max(0, lw - vis_len(strip_ansi(l)))
        rpad = r + " " * max(0, rw - vis_len(strip_ansi(r)))
        out.append(lpad + gap_s + rpad)
    return "\n".join(out)


def split_widths(total: int, *, gap: int = 2, left_ratio: float = 0.58) -> tuple[int, int]:
    """Divide terminal width into two panel outer widths + gap (always fit)."""
    gap = max(0, gap)
    if total <= gap + 16:
        # tiny terminal: still split minimally
        avail = max(0, total - gap)
        left = max(8, avail // 2)
        right = max(8, avail - left)
        while left + right + gap > total and left > 8:
            left -= 1
        while left + right + gap > total and right > 8:
            right -= 1
        return left, right
    avail = total - gap
    left = max(8, int(avail * left_ratio))
    right = max(8, avail - left)
    if left + right + gap > total:
        right = max(8, total - gap - left)
    if left + right + gap > total:
        left = max(8, total - gap - right)
    return left, right


# ---------------------------------------------------------------------------
# shared TUI session layer — one implementation for every pixie TUI
# (siren, scroll, scry, goblin, spellbook, ether): open_tty / screen hold /
# alt screen / cbreak / raw keys / mouse / teardown. Kept here so a single
# fix lands everywhere instead of six copies.
#
# Mouse (SGR 1006): tui_begin enables reporting when mouse=True (default).
# Use tui_read_event() + HitMap for clickable regions. tui_read_key() stays
# string-only and silently discards mouse events so keyboard apps keep working.
# Cell coords are 1-based top-left (terminal SGR convention; paint_frame homes).
# ---------------------------------------------------------------------------

ENTER_ALT = "\033[?1049h\033[?25l"
LEAVE_ALT = "\033[?25h\033[?7h\033[?1049l"
# 1000 = click, 1002 = drag, 1006 = SGR extended coords
MOUSE_ON = "\033[?1000h\033[?1002h\033[?1006h"
MOUSE_OFF = "\033[?1006l\033[?1002l\033[?1000l"
PASTE_ON = "\033[?2004h"
PASTE_OFF = "\033[?2004l"
_TUI_HYGIENE = "\033[0m\033[?25h\033[?7h" + PASTE_OFF + MOUSE_OFF

_tui_fd: int | None = None
_tui_old = None
_tui_alt = False
_tui_mouse = False
_tui_hold_name: str | None = None
_tui_active = False  # True while a tui_begin session is live


@dataclass(frozen=True)
class PasteEvent:
    """Bracketed paste payload (newlines normalized)."""

    text: str


@dataclass(frozen=True)
class MouseEvent:
    """One SGR/X10 mouse report. x/y are 1-based cell coordinates."""

    button: int  # 0 left · 1 middle · 2 right · 64 wheel-up · 65 wheel-down
    action: str  # "press" | "release" | "drag" | "wheel"
    x: int
    y: int
    mods: int = 0  # bit0 shift · bit1 alt · bit2 ctrl (decoded from Pb)
    raw: str = ""


@dataclass
class Region:
    """Clickable rect in 1-based inclusive cell coordinates."""

    id: str
    x0: int
    y0: int
    x1: int
    y1: int
    z: int = 0
    role: str = ""  # list-row | panel | button | slider | scroll | …
    data: object = None

    def contains(self, x: int, y: int) -> bool:
        return self.x0 <= x <= self.x1 and self.y0 <= y <= self.y1


class HitMap:
    """Per-frame registry of clickable regions (highest z wins)."""

    def __init__(self) -> None:
        self._regions: list[Region] = []

    def clear(self) -> None:
        self._regions.clear()

    def add(self, region: Region) -> None:
        self._regions.append(region)

    def hit(self, x: int, y: int) -> Region | None:
        best: Region | None = None
        for r in self._regions:
            if r.contains(x, y) and (best is None or r.z >= best.z):
                best = r
        return best

    def hits(self, x: int, y: int) -> list[Region]:
        found = [r for r in self._regions if r.contains(x, y)]
        found.sort(key=lambda r: r.z, reverse=True)
        return found

    def __len__(self) -> int:
        return len(self._regions)


def add_list_rows(
    hitmap: HitMap,
    *,
    x0: int,
    x1: int,
    y0: int,
    count: int,
    start_index: int = 0,
    id_prefix: str = "row",
    z: int = 0,
    data_at=None,
) -> None:
    """Register count one-line rows starting at y0. data_at(i) optional payload."""
    for i in range(count):
        idx = start_index + i
        payload = data_at(idx) if data_at is not None else idx
        hitmap.add(
            Region(
                id=f"{id_prefix}:{idx}",
                x0=x0,
                y0=y0 + i,
                x1=x1,
                y1=y0 + i,
                z=z,
                role="list-row",
                data=payload,
            )
        )


def wheel_delta(ev: MouseEvent) -> int:
    """+1 wheel-up, -1 wheel-down, else 0."""
    if ev.action != "wheel" and ev.button not in (64, 65):
        return 0
    if ev.button == 64:
        return 1
    if ev.button == 65:
        return -1
    return 0


class DoubleClickTracker:
    """Detect double-clicks on the same region id within `ms` milliseconds."""

    def __init__(self, ms: int = 350) -> None:
        self.ms = ms
        self._last_id: str | None = None
        self._last_t: float = 0.0

    def is_double(self, region_id: str, now: float | None = None) -> bool:
        t = time.monotonic() if now is None else now
        if (
            self._last_id == region_id
            and (t - self._last_t) * 1000.0 <= self.ms
        ):
            self._last_id = None
            self._last_t = 0.0
            return True
        self._last_id = region_id
        self._last_t = t
        return False

    def reset(self) -> None:
        self._last_id = None
        self._last_t = 0.0


def parse_sgr_mouse(seq: str) -> MouseEvent | None:
    """Parse CSI body after '[' for SGR mouse: '<Pb;Px;PyM' or '...m'."""
    if not seq.startswith("<"):
        return None
    final = seq[-1] if seq else ""
    if final not in ("M", "m"):
        return None
    body = seq[1:-1]
    parts = body.split(";")
    if len(parts) != 3:
        return None
    try:
        pb = int(parts[0])
        x = int(parts[1])
        y = int(parts[2])
    except ValueError:
        return None
    mods = 0
    if pb & 4:
        mods |= 1  # shift
    if pb & 8:
        mods |= 2  # alt
    if pb & 16:
        mods |= 4  # ctrl
    base = pb & ~0b11100  # strip mod bits 2,3,4 (values 4,8,16)
    # wheel: 64/65 (and 66/67 for some terminals); drag flag is 32
    if base >= 64:
        button = base  # keep 64/65
        action = "wheel"
    elif final == "m":
        button = base & 3
        action = "release"
    elif base & 32:
        button = base & 3
        action = "drag"
    else:
        button = base & 3
        action = "press"
    return MouseEvent(button=button, action=action, x=x, y=y, mods=mods, raw=seq)


def tui_open_tty() -> int | None:
    """Controlling TTY (or stdin if it is one) for painting + keys."""
    # Try /dev/tty first (standard controlling terminal)
    try:
        return os.open("/dev/tty", os.O_RDWR | os.O_NOCTTY)
    except OSError:
        pass
    # Try stdin/stdout/stderr if they are TTYs
    for fd in (0, 1, 2):
        try:
            if os.isatty(fd):
                return os.dup(fd)
        except Exception:
            pass
    # Try to get controlling terminal via os.ctermid() or os.ttyname
    try:
        cterm = os.ctermid()
        if cterm:
            return os.open(cterm, os.O_RDWR | os.O_NOCTTY)
    except Exception:
        pass
    try:
        for fd in (0, 1, 2):
            name = os.ttyname(fd)
            if name:
                return os.open(name, os.O_RDWR | os.O_NOCTTY)
    except Exception:
        pass
    return None


def tui_screen_hold(on: bool, name: str = "pixie") -> None:
    """pixie-screen hold/release so idle-clearing never eats the UI."""
    global _tui_hold_name
    if not PIXIE_SCREEN.is_file():
        return
    try:
        if on:
            subprocess.run([str(PIXIE_SCREEN), "hold", name], check=False, capture_output=True)
            _tui_hold_name = name
        else:
            if _tui_hold_name:
                subprocess.run([str(PIXIE_SCREEN), "release", _tui_hold_name], check=False, capture_output=True)
                _tui_hold_name = None
            subprocess.run([str(PIXIE_SCREEN), "hold", "grace", "20"], check=False, capture_output=True)
    except OSError:
        pass


def tui_cleanup() -> None:
    """Restore termios, leave alt screen, release hold, close the tty fd.

    Idempotent: safe to call from success path + finally + atexit without
    re-spamming leave-alt / hygiene sequences (which must never hit stdout —
    pickers print selections to stdout for the shell).
    """
    global _tui_fd, _tui_old, _tui_alt, _tui_mouse, _tui_active
    if not _tui_active and _tui_fd is None:
        return
    _tui_active = False
    if _tui_old is not None and _tui_fd is not None:
        try:
            termios.tcsetattr(_tui_fd, termios.TCSADRAIN, _tui_old)
        except (termios.error, OSError):
            pass
    if _tui_fd is not None:
        try:
            # Always emit MOUSE_OFF (in _TUI_HYGIENE) so a crashed enable never sticks.
            tty_write(_tui_fd, (LEAVE_ALT if _tui_alt else "") + _TUI_HYGIENE)
        except OSError:
            pass
    _tui_alt = False
    _tui_mouse = False
    bind_tty(None)
    tui_screen_hold(False)
    if _tui_fd is not None and _tui_fd not in (0, 1, 2):
        try:
            os.close(_tui_fd)
        except OSError:
            pass
    _tui_fd = None
    _tui_old = None
    # Guarantee a cooked terminal for anything that runs after the TUI
    # (editors, sudo, shells). Critical when /dev/tty ≠ stdin fd.
    # Sequences go only to the real TTY — never stdout/stderr.
    force_sane_tty()


def tui_begin(fd: int, hold_name: str = "pixie", *, mouse: bool = True) -> None:
    """Start a TUI session: screen hold, alt screen, cbreak, optional mouse.

    mouse=True (default) enables SGR click/drag reporting. Keyboard-only apps
    that still call tui_read_key() ignore mouse events; prefer tui_read_event()
    + HitMap for clickable UIs.
    """
    global _tui_fd, _tui_old, _tui_alt, _tui_mouse, _tui_active
    _tui_fd = fd
    _tui_active = True
    try:
        _tui_old = termios.tcgetattr(fd)
    except termios.error:
        _tui_old = None
    tui_screen_hold(True, hold_name)
    atexit.register(tui_cleanup)
    bind_tty(fd, color=True)
    try:
        tty_write(fd, ENTER_ALT + PASTE_ON + (MOUSE_ON if mouse else ""))
        _tui_alt = True
        _tui_mouse = bool(mouse)
    except OSError:
        pass
    set_ui_mode(fd)


def _decode_csi_key(s: str) -> str:
    """Map CSI body (after '[') to a stable key name, or csi:<raw>."""
    if s.startswith("A"):
        return "up"
    if s.startswith("B"):
        return "down"
    if s.startswith("C"):
        return "right"
    if s.startswith("D"):
        return "left"
    if s.startswith("Z"):
        return "shift-tab"
    if s.startswith("5~"):
        return "pgup"
    if s.startswith("6~"):
        return "pgdn"
    if s.startswith(("H", "1~", "7~")):
        return "home"
    if s.startswith(("F", "4~", "8~")):
        return "end"
    if s.startswith("3~"):
        return "delete"
    return f"csi:{s}"


def _decode_byte_key(ch: bytes) -> str:
    if ch in (b"\r", b"\n"):
        return "enter"
    if ch == b" ":
        return "space"
    if ch == b"\t":
        return "tab"
    if ch == b"\x03":
        return "ctrl-c"
    if ch in (b"\x04",):
        return "ctrl-d"
    if ch in (b"\x7f", b"\x08"):
        return "backspace"
    if ch in (b"\x15",):
        return "ctrl-u"
    if ch == b"\x12":
        return "ctrl-r"
    if ch == b"\x13":
        return "ctrl-s"
    if ch == b"\x17":
        return "ctrl-w"
    if ch == b"\x01":
        return "ctrl-a"
    if ch == b"\x05":
        return "ctrl-e"
    if ch == b"\x10":
        return "ctrl-p"
    if ch == b"\x0c":
        return "ctrl-l"
    if ch == b"\x16":
        return "ctrl-v"
    if ch == b"\x19":
        return "ctrl-y"
    try:
        return ch.decode("utf-8")
    except UnicodeDecodeError:
        return ""


def tui_read_event(fd: int, timeout: float | None = None) -> str | MouseEvent:
    """One input event: key name (str) or MouseEvent. '' on timeout.

    Mouse requires tui_begin(..., mouse=True). Keys match tui_read_key names.
    """
    if timeout is not None:
        r, _, _ = select.select([fd], [], [], timeout)
        if not r:
            return ""
    ch = os.read(fd, 1)
    if not ch:
        return "esc"
    if ch == b"\x1b":
        r, _, _ = select.select([fd], [], [], 0.05)
        if not r:
            return "esc"
        n1 = os.read(fd, 1)
        if n1 == b"[":
            seq = b""
            while True:
                r, _, _ = select.select([fd], [], [], 0.05)
                if not r:
                    break
                b = os.read(fd, 1)
                if not b:
                    break
                seq += b
                if b[0] >= 0x40:
                    break
            s = seq.decode("latin-1", errors="replace")
            me = parse_sgr_mouse(s)
            if me is not None:
                return me
            if s.startswith("200~"):
                buf = bytearray()
                while True:
                    r, _, _ = select.select([fd], [], [], 5.0)
                    if not r:
                        break
                    chunk = os.read(fd, 4096)
                    if not chunk:
                        break
                    buf.extend(chunk)
                    mark = buf.find(b"\x1b[201~")
                    if mark >= 0:
                        text = buf[:mark].decode("utf-8", "replace")
                        return PasteEvent(text.replace("\r\n", "\n").replace("\r", "\n"))
                return PasteEvent(buf.decode("utf-8", "replace").replace("\r\n", "\n").replace("\r", "\n"))
            return _decode_csi_key(s)
        if n1 == b"O":
            r, _, _ = select.select([fd], [], [], 0.05)
            if r:
                o = os.read(fd, 1)
                return {"A": "up", "B": "down", "C": "right", "D": "left"}.get(
                    o.decode("latin-1", errors="replace"), "esc"
                )
            return "esc"
        return "esc"
    return _decode_byte_key(ch)


def tui_read_key(fd: int, timeout: float | None = None) -> str:
    """One keypress (string names). Mouse events are discarded as ''.

    Prefer tui_read_event() when the app handles mouse. Mapping:
    CSI arrows → up/down/left/right, CSI Z → shift-tab, 5~/6~ → pgup/pgdn,
    H/1~/7~ → home, F/4~/8~ → end, 3~ → delete, Enter/space/tab/ctrl-*,
    lone Esc → esc, else a single decoded char. '' on timeout.
    """
    # Drain mouse reports so keyboard-only loops never see csi:<…M garbage.
    deadline = None if timeout is None else time.monotonic() + timeout
    while True:
        if deadline is None:
            rem = None
        else:
            rem = deadline - time.monotonic()
            if rem <= 0:
                return ""
        ev = tui_read_event(fd, timeout=rem)
        if isinstance(ev, (MouseEvent, PasteEvent)):
            if deadline is None:
                continue
            return ""
        return ev


def tui_suspend() -> None:
    """Restore cooked terminal mid-TUI (for input() prompts)."""
    if _tui_fd is not None:
        try:
            tty_write(_tui_fd, MOUSE_OFF)
        except OSError:
            pass
    if _tui_old is not None and _tui_fd is not None:
        try:
            termios.tcsetattr(_tui_fd, termios.TCSADRAIN, _tui_old)
        except (termios.error, OSError):
            pass
    # Also force a sane cooked mode on the controlling tty — editors and
    # line-input need ICANON+ECHO+ISIG. Restoring _tui_old alone is not
    # enough if stdin is a different fd than the TUI's /dev/tty handle.
    force_sane_tty()


def tui_resume() -> None:
    """Re-enter cbreak after tui_suspend()."""
    if _tui_fd is not None:
        try:
            set_ui_mode(_tui_fd)
            if _tui_mouse:
                tty_write(_tui_fd, MOUSE_ON)
        except (termios.error, OSError):
            pass


def force_sane_tty() -> None:
    """Best-effort cooked terminal for external programs (nano/vim/sudo).

    After a TUI in cbreak, Ctrl keys must be signals again (ISIG) and the
    line discipline must be canonical (ICANON+ECHO). Call this before any
    interactive child that expects a normal terminal.

    CSI leave-alt / show-cursor is written **only** to /dev/tty (or a true
    TTY fd). Never to stdout/stderr — those may carry picker selections
    (`summon` / `scroll` → shell `print -z`) and must stay pure text.
    """
    _HYGIENE = "\033[?25h\033[?7h\033[?1049l\033[0m" + MOUSE_OFF

    def _sane(fd: int) -> None:
        try:
            if not os.isatty(fd):
                return
            attrs = termios.tcgetattr(fd)
            # iflag
            attrs[0] |= termios.BRKINT | termios.ICRNL | termios.IXON
            attrs[0] &= ~(termios.IGNBRK | termios.INLCR | termios.IGNCR | termios.ISTRIP)
            # oflag
            attrs[1] |= termios.OPOST
            if hasattr(termios, "ONLCR"):
                attrs[1] |= termios.ONLCR
            # lflag — the important ones for nano/vim
            attrs[3] |= (
                termios.ISIG
                | termios.ICANON
                | termios.ECHO
                | termios.ECHOE
                | termios.ECHOK
                | termios.IEXTEN
            )
            attrs[3] &= ~getattr(termios, "ECHONL", 0)
            # c_cc: make sure VINTR is Ctrl-C
            try:
                attrs[6][termios.VINTR] = b"\x03"
                attrs[6][termios.VEOF] = b"\x04"
                attrs[6][termios.VKILL] = b"\x15"
            except Exception:
                pass
            termios.tcsetattr(fd, termios.TCSANOW, attrs)
        except (termios.error, OSError, ValueError):
            pass

    # Prefer the controlling TTY for both termios + CSI hygiene.
    tfd = -1
    try:
        tfd = os.open("/dev/tty", os.O_RDWR | os.O_NOCTTY)
    except OSError:
        tfd = -1
    if tfd >= 0:
        try:
            try:
                os.write(tfd, _HYGIENE.encode("ascii"))
            except OSError:
                pass
            _sane(tfd)
        finally:
            if tfd not in (0, 1, 2):
                try:
                    os.close(tfd)
                except OSError:
                    pass
    else:
        # No /dev/tty: only touch stdin/out/err if they *are* TTYs, and only
        # termios — never write CSI to a pipe (would corrupt capture).
        for fd in (0, 1, 2):
            try:
                if os.isatty(fd):
                    _sane(fd)
            except OSError:
                pass
