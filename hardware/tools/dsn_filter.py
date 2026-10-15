"""Prepares the DSN for Freerouting: plane nets (GND, +3V3) are fed by the
fanout vias and the inner planes, so they are removed from routing; their
existing vias and stubs become fixed, net-less obstacles."""
import re
import sys
from pathlib import Path

PLANES = ("/GND", "/+3V3")
src = Path(sys.argv[1])
text = src.read_text()

# Drop (plane ...) definitions (a plane spans until its closing paren).
out, i = [], 0
for m in re.finditer(r"\(plane ", text):
    if m.start() < i:
        continue
    depth, j = 0, m.start()
    while True:
        depth += {"(": 1, ")": -1}.get(text[j], 0)
        j += 1
        if depth == 0:
            break
    out.append(text[i:m.start()])
    i = j
text = "".join(out) + text[i:]


def drop_block(t, head):
    k = t.find(head)
    if k < 0:
        return t
    depth, j = 0, k
    while True:
        depth += {"(": 1, ")": -1}.get(t[j], 0)
        j += 1
        if depth == 0:
            break
    return t[:k] + t[j:]


for n in PLANES:
    text = drop_block(text, f"(net {n}\n")
    text = re.sub(rf"\(net {re.escape(n)}\)\(type route\)", "(type fix)", text)
    text = text.replace(f"(net {n})", "")
    # Remaining mentions are class member lists (possibly multi-line).
    text = re.sub(rf"(?<=\s){re.escape(n)}(?=[\s)])", "", text)
src.write_text(text)
