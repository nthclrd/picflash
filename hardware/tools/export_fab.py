"""Exports JLCPCB fabrication files to hardware/fab/: Gerber + drill (zip),
BOM (Comment, Designator, Footprint, LCSC) and CPL (placement)."""
import csv
import json
import shutil
import subprocess
from collections import defaultdict
from pathlib import Path

HW = Path(__file__).resolve().parent.parent
K = "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli"
PCB = str(HW / "picflash-board.kicad_pcb")
FAB = HW / "fab"
GERB = FAB / "gerber"


def ref_key(ref):
    """C2 before C10."""
    return ref.rstrip("0123456789"), int("0" + "".join(c for c in ref if c.isdigit()))


def run(*args):
    r = subprocess.run([K, *args], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stdout + r.stderr)


shutil.rmtree(FAB, ignore_errors=True)
GERB.mkdir(parents=True)
run("pcb", "export", "gerbers", "--layers",
    "F.Cu,In1.Cu,In2.Cu,B.Cu,F.Paste,B.Paste,F.Silkscreen,B.Silkscreen,F.Mask,B.Mask,Edge.Cuts",
    "--subtract-soldermask", "--no-x2", "-o", str(GERB), PCB)
run("pcb", "export", "drill", "--format", "excellon", "--excellon-units", "mm",
    "--generate-map", "--map-format", "gerberx2", "-o", str(GERB) + "/", PCB)
shutil.make_archive(str(FAB / "picflash-gerber"), "zip", GERB)

# CPL: KiCad's position file, renamed to JLCPCB's columns.
pos = FAB / "pos.csv"
run("pcb", "export", "pos", "--format", "csv", "--units", "mm", "--side", "front",
    "--exclude-dnp", "-o", str(pos), PCB)
parts = json.loads((HW / "parts.json").read_text())
with pos.open() as f, (FAB / "picflash-cpl.csv").open("w", newline="") as out:
    w = csv.writer(out)
    w.writerow(["Designator", "Mid X", "Mid Y", "Layer", "Rotation"])
    for row in csv.DictReader(f):
        if row["Ref"].startswith("TP"):
            continue                     # test points: bare pads
        w.writerow([row["Ref"], row["PosX"] + "mm", row["PosY"] + "mm", "Top", row["Rot"]])
pos.unlink()

# BOM grouped by value + footprint + LCSC.
groups = defaultdict(list)
for ref, p in parts.items():
    if ref.startswith("TP"):
        continue
    groups[(p["value"], p["fp"].split(":")[1], p["lcsc"])].append(ref)
with (FAB / "picflash-bom.csv").open("w", newline="") as out:
    w = csv.writer(out)
    w.writerow(["Comment", "Designator", "Footprint", "LCSC"])
    for (value, fp, lcsc), refs in sorted(groups.items()):
        w.writerow([value, ",".join(sorted(refs, key=ref_key)), fp, lcsc])
missing = sum(1 for k in groups if not k[2])
print(f"fab/ : gerber.zip, {len(groups)} lignes BOM ({missing} sans LCSC), CPL")
