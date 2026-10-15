"""Compares KiCad's exported netlist with hardware/netlist.json."""
import json
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

HW = Path(__file__).resolve().parent.parent
K = "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli"
out = HW / "build" / "picflash-board.xml"
out.parent.mkdir(exist_ok=True)
subprocess.run([K, "sch", "export", "netlist", "--format", "kicadxml", "-o", str(out),
                str(HW / "picflash-board.kicad_sch")], check=True, capture_output=True)
want = json.loads((HW / "netlist.json").read_text())
got = {}
for net in ET.parse(out).getroot().iter("net"):
    for node in net.iter("node"):
        got.setdefault(node.get("ref"), {})[node.get("pin")] = net.get("name").lstrip("/")
errors = []
# Same partition: two pins share a net in KiCad iff they share it in the spec.
by_spec, by_kicad = {}, {}
for ref, pins in want.items():
    for pin, net in pins.items():
        k = got.get(ref, {}).get(pin)
        if k is None:
            errors.append(f"{ref}.{pin}: absent du netlist KiCad")
            continue
        by_spec.setdefault(net, set()).add(k)
        by_kicad.setdefault(k, set()).add(net)
errors += [f"{n} éclaté en {sorted(s)}" for n, s in by_spec.items() if len(s) > 1]
errors += [f"{sorted(s)} fusionnés dans {k}" for k, s in by_kicad.items() if len(s) > 1]
for ref, pins in got.items():
    for pin, net in pins.items():
        if not net.startswith("unconnected") and pin not in want.get(ref, {}):
            errors.append(f"{ref}.{pin} relié à {net} hors spec")
print("\n".join(errors) or f"netlist OK : {len(by_spec)} nets, {sum(map(len, want.values()))} broches")
sys.exit(1 if errors else 0)
