"""Places the footprints of parts.json on a 4-layer board and exports a
Specctra DSN for Freerouting. Run with KiCad's Python:

  /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3 gen_pcb.py
"""
import json
import math
from pathlib import Path

import pcbnew
from pcbnew import FromMM as mm, ToMM

from fanout import fanout

HW = Path(__file__).resolve().parent.parent
FPDIR = "/Applications/KiCad/KiCad.app/Contents/SharedSupport/footprints"
OUT = HW / "picflash-board.kicad_pcb"

W, H = 72.0, 44.0           # board size, mm
OX, OY = 100.0, 100.0       # board origin on the sheet
MARGIN = 0.8                # keep parts this far from the edge
GAP = 0.25                  # courtyard clearance between parts
GLOBAL = {"/GND", "/+3V3", "/+5V", "/+1V1"}
POWER_NETS = ["VBUS", "+5V", "TVDD", "SW", "VPP_RAW"]
HALO = {"U1": 1.5, "U4": 0.6, "Y1": 0.6}   # free ring for fan-out (mm)

parts = json.loads((HW / "parts.json").read_text())

# Fixed placements: ref -> (x, y, rotation°) in board coordinates.
FIXED = {
    "J1": (3.6, 24.0, None),        # USB-C, mouth on the left edge
    "J2": (60.6, 30.9, 180),        # target connector, opening on the right edge
    "U1": (24.0, 21.0, 180),        # RP2040
    "U4": (24.0, 35.5, 0),          # flash, under the QSPI pins
    "Y1": (24.0, 10.5, 0),          # crystal, near XIN/XOUT
    "U3": (10.0, 8.0, 0),           # 3V3 LDO
    "U2": (13.0, 31.5, 90),          # USB ESD
    "F1": (10.0, 38.5, 0),
    "U11": (38.0, 5.0, 0),          # VPP boost
    "L1": (44.5, 4.5, 0),
    "D1": (44.5, 10.5, 0),
    "U12": (40.0, 39.0, 0),         # target VDD LDO
    "SW1": (4.5, 4.0, 0), "SW2": (4.5, 41.0 - 1.5, 0),
    "J3": (15.0, 41.5, 90),
    "U5": (51.0, 26.5, 0), "U6": (51.0, 32.0, 0), "U7": (51.0, 15.5, 0),
    "U8": (51.0, 21.0, 0), "U9": (51.0, 37.5, 0), "U10": (51.0, 9.5, 0),
}
# Decoupling and close companions: placed next to this part first.
NEAR = {**{f"C{i}": "U1" for i in range(6, 18)}, "C1": "U3", "C2": "U3", "C3": "U3",
        "C4": "Y1", "C5": "Y1", "R7": "Y1", "C40": "D1", "C41": "U11", "C50": "U12", "C51": "U12",
        **{f"C{20 + 2 * i}": f"U{5 + i}" for i in range(6)},
        **{f"C{21 + 2 * i}": f"U{5 + i}" for i in range(6)}, "C43": "U11"}


def load(fpid):
    lib, name = fpid.split(":")
    fp = pcbnew.FootprintLoad(f"{FPDIR}/{lib}.pretty", name)
    assert fp, fpid
    return fp


def courtyard(fp):
    poly = fp.GetCourtyard(pcbnew.F_CrtYd)
    bb = poly.BBox() if poly.OutlineCount() else fp.GetBoundingBox(False)
    return (ToMM(bb.GetLeft()) - GAP / 2, ToMM(bb.GetTop()) - GAP / 2,
            ToMM(bb.GetRight()) + GAP / 2, ToMM(bb.GetBottom()) + GAP / 2)


def overlaps(a, b):
    return a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3]


def main():
    board = pcbnew.BOARD()
    board.SetCopperLayerCount(4)
    # Inner layers are solid planes: "power" type keeps the router off them.
    board.SetLayerType(pcbnew.In1_Cu, pcbnew.LT_POWER)
    board.SetLayerType(pcbnew.In2_Cu, pcbnew.LT_POWER)
    ds = board.GetDesignSettings()
    nc = ds.m_NetSettings.GetDefaultNetclass()
    nc.SetTrackWidth(mm(0.2)); nc.SetClearance(mm(0.15))
    ds.m_TrackMinWidth = mm(0.1); ds.m_MinClearance = mm(0.127)
    power = pcbnew.NETCLASS("Power")
    power.SetTrackWidth(mm(0.4)); power.SetClearance(mm(0.2))
    power.SetViaDiameter(mm(0.8)); power.SetViaDrill(mm(0.4)); power.SetPriority(0)
    ds.m_NetSettings.SetNetclass("Power", power)
    for n in POWER_NETS:
        ds.m_NetSettings.SetNetclassPatternAssignment("/" + n, "Power")
    nc.SetViaDiameter(mm(0.6)); nc.SetViaDrill(mm(0.3))

    nets = {}
    for p in parts.values():
        for n in p["nets"].values():
            if n not in nets:
                # Local labels on the root sheet: KiCad names the net "/NAME".
                nets[n] = pcbnew.NETINFO_ITEM(board, "/" + n)
                board.Add(nets[n])

    ds.m_NetSettings.RecomputeEffectiveNetclasses()
    for n in POWER_NETS:
        nets[n].SetNetClass(ds.m_NetSettings.GetNetClassByName("Power"))

    # Outline.
    pts = [(0, 0), (W, 0), (W, H), (0, H)]
    for a, b in zip(pts, pts[1:] + pts[:1]):
        seg = pcbnew.PCB_SHAPE(board)
        seg.SetShape(pcbnew.SHAPE_T_SEGMENT)
        seg.SetStart(pcbnew.VECTOR2I(mm(OX + a[0]), mm(OY + a[1])))
        seg.SetEnd(pcbnew.VECTOR2I(mm(OX + b[0]), mm(OY + b[1])))
        seg.SetLayer(pcbnew.Edge_Cuts); seg.SetWidth(mm(0.1))
        board.Add(seg)

    fps, boxes = {}, []

    def put(ref, x, y, rot):
        fp = fps[ref]
        fp.SetOrientationDegrees(rot)
        fp.SetPosition(pcbnew.VECTOR2I(mm(OX + x), mm(OY + y)))

    for ref, p in sorted(parts.items()):
        fp = load(p["fp"])
        fp.SetFPID(pcbnew.LIB_ID(*p["fp"].split(":")))
        fp.SetReference(ref); fp.SetValue(p["value"])
        if p["lcsc"]:
            fp.SetField("LCSC", p["lcsc"])
            fp.GetField("LCSC").SetVisible(False)
        if ref[0] in "RC" and ref[1].isdigit():
            fp.Reference().SetLayer(pcbnew.F_Fab)   # too dense for silk
        else:
            fp.Reference().SetTextSize(pcbnew.VECTOR2I(mm(0.8), mm(0.8)))
            fp.Reference().SetTextThickness(mm(0.12))
        fp.SetPath(pcbnew.KIID_PATH("/" + p["uuid"]))
        for pad in fp.Pads():
            n = p["nets"].get(pad.GetNumber())
            if n:
                pad.SetNet(nets[n])
        board.Add(fp)
        fps[ref] = fp

    # USB-C: pick the rotation that puts the mouth (+y of the footprint) left.
    for rot in (0, 90, 180, 270):
        put("J1", *FIXED["J1"][:2], rot)
        if sum(ToMM(pd.GetPosition().x) for pd in fps["J1"].Pads()) / len(list(fps["J1"].Pads())) > OX + FIXED["J1"][0] + 1:
            break
    placed = {"J1"}
    boxes.append(("J1", courtyard(fps["J1"])))
    for ref, (x, y, rot) in FIXED.items():
        if ref == "J1":
            continue
        put(ref, x, y, rot)
        box = courtyard(fps[ref])
        clash = [r for r, b in boxes if overlaps(box, b)]
        assert not clash, (ref, box, clash, [b for r, b in boxes if r in clash])
        assert box[0] >= OX - 0.01 and box[2] <= OX + W + 12 and box[1] >= OY and box[3] <= OY + H, (ref, box)
        boxes.append((ref, box))
        placed.add(ref)

    netpos = {}

    def register(ref):
        for pd in fps[ref].Pads():
            if pd.GetNetname():
                netpos.setdefault(pd.GetNetname(), []).append((ToMM(pd.GetPosition().x), ToMM(pd.GetPosition().y)))

    for r in placed:
        register(r)

    def anchors(ref):
        """Pad positions this part should sit close to."""
        if ref in NEAR:
            other = fps[NEAR[ref]]
            mine = {"/" + n for n in parts[ref]["nets"].values()}
            pads = [pd for pd in other.Pads() if pd.GetNetname() in mine and pd.GetNetname() != "/GND"]
            return [(ToMM(pd.GetPosition().x), ToMM(pd.GetPosition().y)) for pd in pads] or \
                [(ToMM(other.GetPosition().x), ToMM(other.GetPosition().y))]
        return [q for n in {"/" + n for n in parts[ref]["nets"].values()} - GLOBAL for q in netpos.get(n, [])]

    def shape(ref, rot):
        """Courtyard box and pad offsets relative to the footprint origin."""
        put(ref, 0, 0, rot)
        b = courtyard(fps[ref])
        pads = [(pd.GetNetname(), ToMM(pd.GetPosition().x) - OX, ToMM(pd.GetPosition().y) - OY)
                for pd in fps[ref].Pads() if pd.GetNetname() and pd.GetNetname() not in GLOBAL]
        return (b[0] - OX, b[1] - OY, b[2] - OX, b[3] - OY), pads

    halos = [(b[0] - HALO[r], b[1] - HALO[r], b[2] + HALO[r], b[3] + HALO[r]) for r, b in boxes if r in HALO]
    unplaced = []
    todo = [r for r in parts if r not in placed]
    while todo:
        todo.sort(key=lambda r: (r not in NEAR, -len(anchors(r)), r))
        ref = todo.pop(0)
        a = anchors(ref)
        if not a and any(anchors(r) for r in todo):
            todo.append(ref)
            continue
        if not a:
            a = [(OX + W / 2, OY + H / 2)]
        tx, ty = sum(p[0] for p in a) / len(a) - OX, sum(p[1] for p in a) / len(a) - OY
        shapes = {rot: shape(ref, rot) for rot in (0, 90)}
        best = None
        for radius in [i * 0.5 for i in range(0, 100)]:
            steps = max(1, int(2 * math.pi * radius / 0.5))
            for k in range(steps):
                ang = 2 * math.pi * k / steps
                x, y = round(tx + radius * math.cos(ang), 2), round(ty + radius * math.sin(ang), 2)
                for rot, (b, pads) in shapes.items():
                    box = (OX + x + b[0], OY + y + b[1], OX + x + b[2], OY + y + b[3])
                    if box[0] < OX + MARGIN or box[2] > OX + W - MARGIN or box[1] < OY + MARGIN or box[3] > OY + H - MARGIN:
                        continue
                    if any(overlaps(box, bb) for _, bb in boxes):
                        continue
                    if any(overlaps(box, bb) for bb in halos):
                        continue
                    wl = sum(min((math.dist((OX + x + px, OY + y + py), q) for q in netpos.get(n, [])), default=0)
                             for n, px, py in pads) + radius * 0.3
                    if best is None or wl < best[0]:
                        best = (wl, x, y, rot, box)
            if best and radius >= 4:
                break
        if not best:
            unplaced.append(ref)
            put(ref, W + 10, 5 + 3 * len(unplaced), 0)
            continue
        _, x, y, rot, box = best
        put(ref, x, y, rot)
        boxes.append((ref, box))
        placed.add(ref)
        register(ref)

    (HW / "build").mkdir(exist_ok=True)
    (HW / "build" / "fanout.json").write_text(json.dumps(fanout(board, OX, OY, W, H)))

    # Planes: GND on In1, +3V3 on In2. The outer GND pours come after routing
    # (finish_pcb.py): Freerouting would treat them as obstacles.
    for layer, net in [(pcbnew.In1_Cu, "GND"), (pcbnew.In2_Cu, "+3V3")]:
        z = pcbnew.ZONE(board)
        z.SetLayer(layer); z.SetNet(nets[net]); z.SetAssignedPriority(0)
        z.SetLocalClearance(mm(0.25)); z.SetMinThickness(mm(0.2))
        z.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
        ol = z.Outline(); ol.NewOutline()
        for x, y in [(0.3, 0.3), (W - 0.3, 0.3), (W - 0.3, H - 0.3), (0.3, H - 0.3)]:
            ol.Append(mm(OX + x), mm(OY + y))
        board.Add(z)

    board.SetFileName(str(OUT))
    pcbnew.SaveBoard(str(OUT), board)
    assert pcbnew.ExportSpecctraDSN(board, str(HW / "build" / "picflash-board.dsn"))
    print(f"{OUT.name}: {len(fps) - len(unplaced)} empreintes placées, hors carte : {unplaced}")


main()
