"""Imports Freerouting's session, adds outer GND pours, fills zones."""
import json
from pathlib import Path

import pcbnew
from pcbnew import FromMM as mm

from fanout import fanout

HW = Path(__file__).resolve().parent.parent
PCB = HW / "picflash-board.kicad_pcb"
W, H, OX, OY = 72.0, 44.0, 100.0, 100.0

board = pcbnew.LoadBoard(str(PCB))
assert pcbnew.ImportSpecctraSES(board, str(HW / "build" / "picflash-board.ses"))
# The session import replaces every track: lay the plane fan-out again, now
# avoiding the routed signals (the router kept these spots free).
fanout(board, OX, OY, W, H, replay=json.loads((HW / "build" / "fanout.json").read_text()))
gnd = board.FindNet("/GND")
for layer in (pcbnew.F_Cu, pcbnew.B_Cu):
    z = pcbnew.ZONE(board)
    z.SetLayer(layer); z.SetNet(gnd); z.SetAssignedPriority(0)
    z.SetLocalClearance(mm(0.25)); z.SetMinThickness(mm(0.2))
    # Solid connections: thermal spokes get starved on this dense board.
    z.SetPadConnection(pcbnew.ZONE_CONNECTION_FULL)
    ol = z.Outline(); ol.NewOutline()
    for x, y in [(0.3, 0.3), (W - 0.3, 0.3), (W - 0.3, H - 0.3), (0.3, H - 0.3)]:
        ol.Append(mm(OX + x), mm(OY + y))
    board.Add(z)
for z in board.Zones():
    if z.GetLayer() in (pcbnew.In1_Cu, pcbnew.In2_Cu):
        z.SetPadConnection(pcbnew.ZONE_CONNECTION_FULL)   # inner planes: solid
pcbnew.ZONE_FILLER(board).Fill(board.Zones())
pcbnew.SaveBoard(str(PCB), board)
print(f"{PCB.name}: {len(board.GetTracks())} pistes/vias, {len(list(board.Zones()))} zones")
