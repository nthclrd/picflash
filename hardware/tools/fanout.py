"""Fan-out vias from SMD pads on plane nets (GND, +3V3) to the inner planes."""
import math

import pcbnew
from pcbnew import FromMM as mm, ToMM


def fanout(board, OX, OY, W, H, replay=None):
    """Via + short stub next to every SMD pad on a plane net, so the planes
    feed them directly and the router only handles signals."""
    VIA_D, VIA_DRILL, CLR, STUB_W = 0.6, 0.3, 0.16, 0.2
    planes = {"/GND", "/+3V3"}
    obstacles = []   # (net, x0, y0, x1, y1)
    for fp in board.GetFootprints():
        for pd in fp.Pads():
            bb = pd.GetBoundingBox()
            obstacles.append((pd.GetNetname(), ToMM(bb.GetLeft()), ToMM(bb.GetTop()),
                              ToMM(bb.GetRight()), ToMM(bb.GetBottom())))
    segs = []        # (net, x0, y0, x1, y1, half width) of existing tracks
    for t in board.GetTracks():
        if t.Type() == pcbnew.PCB_VIA_T:
            p = t.GetPosition()
            obstacles.append((t.GetNetname(), ToMM(p.x) - ToMM(t.GetWidth(pcbnew.F_Cu)) / 2,
                              ToMM(p.y) - ToMM(t.GetWidth(pcbnew.F_Cu)) / 2,
                              ToMM(p.x) + ToMM(t.GetWidth(pcbnew.F_Cu)) / 2,
                              ToMM(p.y) + ToMM(t.GetWidth(pcbnew.F_Cu)) / 2))
        elif t.GetLayer() in (pcbnew.F_Cu, pcbnew.B_Cu):
            segs.append((t.GetNetname(), ToMM(t.GetStart().x), ToMM(t.GetStart().y),
                         ToMM(t.GetEnd().x), ToMM(t.GetEnd().y), ToMM(t.GetWidth()) / 2, t.GetLayer()))
    vias = []        # (net, x, y)

    def seg_dist(x, y, s):
        _, ax, ay, bx, by, *_ = s
        dx, dy = bx - ax, by - ay
        L = dx * dx + dy * dy
        u = 0 if L == 0 else max(0, min(1, ((x - ax) * dx + (y - ay) * dy) / L))
        return math.hypot(x - ax - u * dx, y - ay - u * dy)

    def rect_dist(x, y, r):
        dx = max(r[1] - x, 0, x - r[3])
        dy = max(r[2] - y, 0, y - r[4])
        return math.hypot(dx, dy)

    def free(net, x, y, x0, y0):
        if not (OX + 0.8 < x < OX + W - 0.8 and OY + 0.8 < y < OY + H - 0.8):
            return False
        for r in obstacles:
            if r[0] == net:
                continue
            if rect_dist(x, y, r) < VIA_D / 2 + CLR:
                return False
            # stub from pad centre to via
            for t in [i / 10 for i in range(1, 10)]:
                if rect_dist(x0 + (x - x0) * t, y0 + (y - y0) * t, r) < STUB_W / 2 + CLR:
                    return False
        for sg in segs:
            if sg[0] == net:
                continue
            # The via blocks both outer layers; the stub only F.Cu.
            if seg_dist(x, y, sg) < VIA_D / 2 + CLR + sg[5]:
                return False
            if sg[6] == pcbnew.F_Cu and any(
                    seg_dist(x0 + (x - x0) * t, y0 + (y - y0) * t, sg) < STUB_W / 2 + CLR + sg[5]
                    for t in [i / 10 for i in range(1, 10)]):
                return False
        return all(math.hypot(x - vx, y - vy) >= (VIA_D + CLR if n != net else VIA_D + 0.1)
                   for n, vx, vy in vias)

    def add_via(net, x, y):
        v = pcbnew.PCB_VIA(board)
        v.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
        v.SetWidth(mm(VIA_D)); v.SetDrill(mm(VIA_DRILL)); v.SetNet(board.FindNet(net))
        board.Add(v)
        vias.append((net, x, y))
        obstacles.append((net, x - VIA_D / 2, y - VIA_D / 2, x + VIA_D / 2, y + VIA_D / 2))

    placed = []      # (net, pad x, pad y, via x, via y) for replay
    if replay is not None:
        # Re-create exactly the fan-out the router was given.
        for net, x0, y0, x, y in replay:
            add_via(net, x, y)
            if (x0, y0) != (x, y):
                t = pcbnew.PCB_TRACK(board)
                t.SetStart(pcbnew.VECTOR2I(mm(x0), mm(y0))); t.SetEnd(pcbnew.VECTOR2I(mm(x), mm(y)))
                t.SetWidth(mm(STUB_W)); t.SetLayer(pcbnew.F_Cu); t.SetNet(board.FindNet(net))
                board.Add(t)
        print(f"fanout rejoué : {len(replay)} vias")
        return replay
    done = missing = 0
    missed = []
    for fp in board.GetFootprints():
        for pd in fp.Pads():
            net = pd.GetNetname()
            if net not in planes or pd.GetAttribute() != pcbnew.PAD_ATTRIB_SMD:
                continue
            x0, y0 = ToMM(pd.GetPosition().x), ToMM(pd.GetPosition().y)
            bb = pd.GetBoundingBox()
            if ToMM(bb.GetWidth()) > 2.5 and ToMM(bb.GetHeight()) > 2.5:
                # Exposed pad: 3x3 vias inside it.
                for i in (-1, 0, 1):
                    for j in (-1, 0, 1):
                        add_via(net, x0 + i * 1.0, y0 + j * 1.0)
                        placed.append((net, x0 + i * 1.0, y0 + j * 1.0, x0 + i * 1.0, y0 + j * 1.0))
                done += 1
                continue
            reach = max(ToMM(bb.GetWidth()), ToMM(bb.GetHeight())) / 2
            spot = None
            for d in [reach + 0.45 + 0.1 * k for k in range(25)]:
                for a in range(0, 360, 15):
                    x, y = x0 + d * math.cos(math.radians(a)), y0 + d * math.sin(math.radians(a))
                    if free(net, x, y, x0, y0):
                        spot = (x, y)
                        break
                if spot:
                    break
            if not spot:
                missing += 1
                missed.append(f"{fp.GetReference()}.{pd.GetNumber()}({net[1:]})")
                continue
            add_via(net, *spot)
            placed.append((net, x0, y0, *spot))
            t = pcbnew.PCB_TRACK(board)
            t.SetStart(pcbnew.VECTOR2I(mm(x0), mm(y0))); t.SetEnd(pcbnew.VECTOR2I(mm(spot[0]), mm(spot[1])))
            t.SetWidth(mm(STUB_W)); t.SetLayer(pd.GetLayer() if pd.IsOnLayer(pcbnew.F_Cu) else pcbnew.F_Cu)
            t.SetNet(board.FindNet(net))
            board.Add(t)
            done += 1
    print(f"fanout : {done} pastilles reliées aux plans ; sans via : {', '.join(missed)}")
    return placed
