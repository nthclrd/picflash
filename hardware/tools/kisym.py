"""Minimal KiCad s-expression reader/writer and symbol-library access."""
import copy
import re
from pathlib import Path

LIBDIR = Path("/Applications/KiCad/KiCad.app/Contents/SharedSupport/symbols")
_TOK = re.compile(r'\s*(\(|\)|"(?:[^"\\]|\\.)*"|[^\s()"]+)')


class Sym(str):
    """Bare atom (unquoted)."""


def parse(text):
    stack, cur = [], []
    pos = 0
    while True:
        m = _TOK.match(text, pos)
        if not m:
            break
        pos = m.end()
        t = m.group(1)
        if t == "(":
            stack.append(cur)
            cur = []
        elif t == ")":
            done = cur
            cur = stack.pop()
            cur.append(done)
        elif t.startswith('"'):
            cur.append(t[1:-1].replace('\\"', '"').replace("\\\\", "\\"))
        else:
            cur.append(Sym(t))
    return cur[0]


def dump(x, indent=0):
    if isinstance(x, list):
        inner = [dump(e, indent + 1) for e in x]
        flat = "(" + " ".join(inner) + ")"
        if len(flat) < 100 and "\n" not in flat:
            return flat
        pad = "\n" + "  " * (indent + 1)
        return "(" + inner[0] + pad + pad.join(inner[1:]) + ")"
    if isinstance(x, Sym):
        return str(x)
    if isinstance(x, (int, float)):
        return f"{x:g}" if isinstance(x, float) else str(x)
    return '"' + str(x).replace("\\", "\\\\").replace('"', '\\"') + '"'


def find(node, head):
    return [e for e in node if isinstance(e, list) and e and e[0] == head]


def first(node, head):
    r = find(node, head)
    return r[0] if r else None


_cache = {}


def library(lib):
    if lib not in _cache:
        _cache[lib] = {s[1]: s for s in find(parse((LIBDIR / f"{lib}.kicad_sym").read_text()), "symbol")}
    return _cache[lib]


def symbol(libid):
    """Flattened copy of a library symbol, renamed to its lib id."""
    lib, name = libid.split(":")
    s = copy.deepcopy(library(lib)[name])
    ext = first(s, "extends")
    if ext:
        parent = copy.deepcopy(library(lib)[ext[1]])
        props = {p[1]: p for p in find(s, "property")}
        out = [Sym("symbol"), libid]
        for e in parent[2:]:
            if isinstance(e, list) and e[0] == "property" and e[1] in props:
                out.append(props.pop(e[1]))
            elif isinstance(e, list) and e[0] == "symbol":
                e[1] = e[1].replace(ext[1], name, 1)
                out.append(e)
            else:
                out.append(e)
        out[3:3] = list(props.values())
        s = out
    s[1] = libid
    return s


def pins(sym):
    """[(number, name, type, x, y, angle, unit)] in symbol coordinates (y up)."""
    out = []
    for sub in find(sym, "symbol"):
        m = re.search(r"_(\d+)_(\d+)$", sub[1])
        unit = int(m.group(1)) if m else 1
        for p in find(sub, "pin"):
            at = first(p, "at")
            out.append((first(p, "number")[1], first(p, "name")[1], str(p[1]),
                        float(at[1]), float(at[2]), float(at[3]) if len(at) > 3 else 0.0, unit))
    return out
