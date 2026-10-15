"""Generates hardware/picflash-board.kicad_sch from the netlist below.

Connections are made with net labels on a short wire stub at every pin, so
the netlist here is the single source of truth. Run, then check with
`kicad-cli sch erc` and `check_netlist.py`.
"""
import copy
import json
import sys
import uuid
from pathlib import Path

import kisym
from kisym import Sym

OUT = Path(__file__).resolve().parent.parent / "picflash-board.kicad_sch"
NS = uuid.UUID("5f1c0b52-7d0e-4a8e-9a43-7c2d1f0b9e11")
ROOT = str(uuid.uuid5(NS, "root"))
STUB = 2.54

R0603 = "Resistor_SMD:R_0603_1608Metric"
C0603 = "Capacitor_SMD:C_0603_1608Metric"
C0805 = "Capacitor_SMD:C_0805_2012Metric"
C1206 = "Capacitor_SMD:C_1206_3216Metric"
LED0603 = "LED_SMD:LED_0603_1608Metric"
SOT23_6 = "Package_TO_SOT_SMD:SOT-23-6"

parts = []  # dicts: ref, lib, value, fp, lcsc, at, nets{pin: net}


def part(ref, lib, value, fp, at, nets, lcsc=""):
    parts.append(dict(ref=ref, lib=lib, value=value, fp=fp, lcsc=lcsc, at=at, nets=nets))


def two(ref, lib, value, fp, at, a, b, lcsc=""):
    part(ref, lib, value, fp, at, {"1": a, "2": b}, lcsc)


def r(ref, value, at, a, b):
    two(ref, "Device:R", value, R0603, at, a, b)


def c(ref, value, at, a, b="GND", fp=C0603):
    two(ref, "Device:C", value, fp, at, a, b)


def row(x, y, items, step=7.62):
    """Places 2-pin parts left to right: items = (kind, ref, value, a, b[, fp])."""
    for i, it in enumerate(items):
        kind, ref, value, a, b, *fp = it
        at = (x + i * step, y)
        if kind == "R":
            r(ref, value, at, a, b)
        else:
            c(ref, value, at, a, b, *(fp or [C0603]))


notes = []  # (x, y, text)
# --- USB-C, protection --------------------------------------------------
notes.append((20, 22, "USB-C : 5,1k sur CC (puits), ESD USBLC6, polyfuse 500 mA, 27R série D+/D-"))
part("J1", "Connector:USB_C_Receptacle_USB2.0_16P", "USB-C", "Connector_USB:USB_C_Receptacle_HRO_TYPE-C-31-M-12",
     (40.64, 63.5), {**{p: "VBUS" for p in ("A4", "A9", "B4", "B9")},
                     **{p: "GND" for p in ("A1", "A12", "B1", "B12", "SH")},
                     "A5": "CC1", "B5": "CC2", "A6": "USB_DP_C", "B6": "USB_DP_C",
                     "A7": "USB_DM_C", "B7": "USB_DM_C", "A8": None, "B8": None}, "C165948")
part("U2", "Power_Protection:USBLC6-2SC6", "USBLC6-2SC6", SOT23_6, (96.52, 50.8),
     {"1": "USB_DP_C", "6": "USB_DP_C", "3": "USB_DM_C", "4": "USB_DM_C", "5": "VBUS", "2": "GND"}, "C7519")
two("F1", "Device:Polyfuse", "500mA", "Fuse:Fuse_1206_3216Metric", (127, 50.8), "VBUS", "+5V")
row(81.28, 76.2, [("R", "R1", "5.1k", "CC1", "GND"), ("R", "R2", "5.1k", "CC2", "GND"),
                  ("R", "R3", "27", "USB_DP_C", "USB_DP"), ("R", "R4", "27", "USB_DM_C", "USB_DM"),
                  ("R", "R5", "10k", "+5V", "VBUS_SENSE"), ("R", "R6", "10k", "VBUS_SENSE", "GND")])

# --- 3V3 ------------------------------------------------------------------
notes.append((20, 102, "3,3 V logique (AP2112K, 600 mA)"))
part("U3", "Regulator_Linear:AP2112K-3.3", "AP2112K-3.3", "Package_TO_SOT_SMD:SOT-23-5", (50.8, 121.92),
     {"1": "+5V", "3": "+5V", "2": "GND", "4": None, "5": "+3V3"}, "C51118")
row(76.2, 121.92, [("C", "C1", "1u", "+5V", "GND"), ("C", "C2", "10u", "+3V3", "GND", C0805),
                   ("C", "C3", "22u 10V", "+5V", "GND", C0805)])

# --- RP2040 ---------------------------------------------------------------
notes.append((150, 22, "RP2040 + flash QSPI 16 Mbit + quartz 12 MHz"))
gp = {0: "DBG_TX", 1: "DBG_RX", 2: "PGC_A", 3: "PGC_DIR", 4: "PGD_A", 5: "PGD_DIR", 6: "AUX_A",
      7: "AUX_DIR", 8: "TDI_A", 9: "TDI_DIR", 10: "TMS_A", 11: "TMS_DIR", 12: "MCLR_LV_A",
      13: "VPP_ON", 14: "MCLR_PD", 15: "BOOST_EN", 16: "VPP_PWM", 17: "VT_EN", 18: "VT_PWM",
      20: "LED_ACT", 21: "LED_ERR", 26: "VT_ADC", 27: "VPP_SENSE", 28: "VBUS_SENSE"}
gpio_pin = {0: 2, 1: 3, 2: 4, 3: 5, 4: 6, 5: 7, 6: 8, 7: 9, 8: 11, 9: 12, 10: 13, 11: 14, 12: 15, 13: 16,
            14: 17, 15: 18, 16: 27, 17: 28, 18: 29, 19: 30, 20: 31, 21: 32, 22: 34, 23: 35, 24: 36,
            25: 37, 26: 38, 27: 39, 28: 40, 29: 41}
u1 = {str(gpio_pin[g]): gp.get(g) for g in gpio_pin}
u1.update({p: "+3V3" for p in ("1", "10", "22", "33", "42", "49", "43", "44", "48")})
u1.update({"23": "+1V1", "50": "+1V1", "45": "+1V1", "57": "GND", "19": "GND", "20": "XIN",
           "21": "XOUT", "24": "SWCLK", "25": "SWDIO", "26": "RUN", "46": "USB_DM", "47": "USB_DP",
           "51": "QSPI_SD3", "52": "QSPI_SCLK", "53": "QSPI_SD0", "54": "QSPI_SD2", "55": "QSPI_SD1",
           "56": "QSPI_SS"})
part("U1", "MCU_RaspberryPi:RP2040", "RP2040", None, (200.66, 76.2), u1, "C2040")
part("U4", "Memory_Flash:W25Q16JVSS", "W25Q16JVSSIQ", None, (271.78, 60.96),
     {"1": "QSPI_SS", "2": "QSPI_SD1", "3": "QSPI_SD2", "4": "GND", "5": "QSPI_SD0", "6": "QSPI_SCLK",
      "7": "QSPI_SD3", "8": "+3V3"})
part("Y1", "Device:Crystal_GND24", "12MHz", "Crystal:Crystal_SMD_3225-4Pin_3.2x2.5mm", (271.78, 99.06),
     {"1": "XIN", "3": "XOUT_X", "2": "GND", "4": "GND"}, "C20625731")   # ABM8-272-T3, 10 pF load
row(254, 119.38, [("C", "C4", "15p", "XIN", "GND"), ("C", "C5", "15p", "XOUT_X", "GND"),
                  ("R", "R7", "1k", "XOUT", "XOUT_X"), ("R", "R8", "1k", "BOOT_R", "GND")])
part("SW1", "Switch:SW_Push", "BOOTSEL", "Button_Switch_SMD:SW_SPST_PTS810", (304.8, 119.38),
     {"1": "QSPI_SS", "2": "BOOT_R"})
part("SW2", "Switch:SW_Push", "RESET", "Button_Switch_SMD:SW_SPST_PTS810", (304.8, 132.08),
     {"1": "RUN", "2": "GND"})
row(152.4, 139.7, [("C", f"C{6 + i}", "100n", "+3V3", "GND") for i in range(8)]
    + [("C", "C14", "1u", "+3V3", "GND"), ("C", "C15", "1u", "+1V1", "GND"),
       ("C", "C16", "100n", "+1V1", "GND"), ("C", "C17", "100n", "+1V1", "GND"),
       ("R", "R9", "10k", "+3V3", "RUN")])
part("J3", "Connector_Generic:Conn_01x03", "SWD", "Connector_PinHeader_1.27mm:PinHeader_1x03_P1.27mm_Vertical",
     (320.04, 60.96), {"1": "SWCLK", "2": "GND", "3": "SWDIO"})
for i, net in enumerate(["DBG_TX", "DBG_RX", "VPP_RAW", "TVDD", "GND"]):
    part(f"TP{i + 1}", "Connector:TestPoint", net, "TestPoint:TestPoint_Pad_D1.0mm",
         (320.04 + 7.62 * i, 83.82), {"1": net})

# --- LEDs -----------------------------------------------------------------
for i, (led, sig, color) in enumerate([("D2", "+3V3", "green"), ("D3", "LED_ACT", "yellow"),
                                       ("D4", "LED_ERR", "red")]):
    x = 345.44 + 22.86 * i
    r(f"R{10 + i}", "330", (x, 119.38), sig, f"{led}_A")
    two(led, "Device:LED", color, LED0603, (x, 134.62), "GND", f"{led}_A")

# --- Level shifters ---------------------------------------------------------
notes.append((20, 192, "Lignes ICSP : SN74LVC1T45 (1,65-5,5 V côté cible). DIR tiré à 0 = côté cible en"))
notes.append((20, 197, "entrée (haute impédance) au démarrage. 100R série vers le connecteur."))
for i, (line, out) in enumerate([("PGC", "ICSP_PGC"), ("PGD", "ICSP_PGD"), ("AUX", "ICSP_AUX"),
                                 ("TDI", "ICSP_TDI"), ("TMS", "ICSP_TMS"), ("MCLR_LV", None)]):
    x, y = 30.48 + 63.5 * i, 220.98
    part(f"U{5 + i}", "Logic_LevelTranslator:SN74LVC1T45DBV", "SN74LVC1T45DBV", None, (x, y),
         {"1": "+3V3", "2": "GND", "3": f"{line}_A", "4": f"{line}_B", "5": f"{line}_DIR", "6": "TVDD"})
    if out:
        row(x - 12.7, y + 27.94, [("R", f"R{20 + 2 * i}", "100", f"{line}_B", out),
                                 ("R", f"R{21 + 2 * i}", "10k", f"{line}_DIR", "GND"),
                                 ("C", f"C{20 + 2 * i}", "100n", "+3V3", "GND"),
                                 ("C", f"C{21 + 2 * i}", "100n", "TVDD", "GND")])
    else:
        row(x - 12.7, y + 27.94, [("R", "R30", "100k", "MCLR_LV_A", "GND"),
                                 ("C", "C30", "100n", "+3V3", "GND"),
                                 ("C", "C31", "100n", "TVDD", "GND"),
                                 ("R", "R31", "10k", "+3V3", "MCLR_LV_DIR")])
        two("D5", "Device:D_Schottky", "BAT54WS", "Diode_SMD:D_SOD-323", (x + 22.86, y + 27.94),
            "MCLR", "MCLR_LV_B")

# --- VPP boost and MCLR switching -----------------------------------------
notes.append((20, 150, "VPP : MT3608, 6-14 V réglé par PWM filtré injecté dans FB, mesuré sur ADC1."))
part("U11", "Regulator_Switching:MT3608", "MT3608", SOT23_6, (50.8, 170.18),
     {"5": "+5V", "2": "GND", "4": "BOOST_EN", "1": "SW", "3": "VPP_FB", "6": None}, "C84817")
c("C43", "10u", (27.94, 170.18), "+5V", "GND", C0805)   # MT3608 input
two("L1", "Device:L", "22uH", "Inductor_SMD:L_Cenker_CKCS4030", (78.74, 160.02), "+5V", "SW")
two("D1", "Device:D_Schottky", "SS34", "Diode_SMD:D_SMA", (99.06, 160.02), "VPP_RAW", "SW", "C8678")
row(109.22, 170.18, [("C", "C40", "22u 25V", "VPP_RAW", "GND", C1206), ("R", "R40", "200k", "VPP_RAW", "VPP_FB"),
                    ("R", "R41", "10k", "VPP_FB", "GND"), ("R", "R42", "82k", "VPP_FB", "VPP_DAC"),
                    ("R", "R43", "10k", "VPP_PWM", "VPP_DAC"), ("C", "C41", "1u", "VPP_DAC", "GND"),
                    ("R", "R44", "100k", "BOOST_EN", "GND"), ("R", "R45", "47k", "VPP_RAW", "GND"),
                    ("R", "R46", "100k", "VPP_RAW", "VPP_SENSE"), ("R", "R47", "22k", "VPP_SENSE", "GND"),
                    ("C", "C42", "100n", "VPP_SENSE", "GND")])
notes.append((200, 197, "MCLR : Q1 = VPP, D5 = VDD (LVP), Q3 = masse, sinon relâché"))
part("Q1", "Transistor_FET:AO3401A", "AO3401A", None, (210.82, 170.18),
     {"1": "VPP_G", "2": "VPP_RAW", "3": "VPP_SW"}, "C15127")
r("R53", "220", (193.04, 170.18), "VPP_SW", "MCLR")
part("Q2", "Transistor_FET:2N7002", "2N7002", None, (241.3, 170.18),
     {"1": "VPP_ON", "2": "GND", "3": "VPP_GD"}, "C8545")
part("Q3", "Transistor_FET:2N7002", "2N7002", None, (271.78, 170.18),
     {"1": "MCLR_PD", "2": "GND", "3": "MCLR"}, "C8545")
row(287.02, 170.18, [("R", "R48", "10k", "VPP_RAW", "VPP_G"), ("R", "R49", "10k", "VPP_G", "VPP_GD"),
                     ("R", "R50", "100k", "VPP_ON", "GND"), ("R", "R51", "100k", "MCLR_PD", "GND"),
                     ("R", "R52", "100", "MCLR", "ICSP_MCLR")])

# --- Target VDD -----------------------------------------------------------
notes.append((322, 142, "VDD cible : TLV75801, 1,5-4,85 V"))
notes.append((322, 147, "PWM sur FB, EN tiré à 0, ADC0"))
part("U12", "Regulator_Linear:TLV75801PDBV", "TLV75801PDBV", None, (342.9, 172.72),
     {"1": "+5V", "2": "GND", "3": "VT_EN", "4": "VT_FB", "5": "TVDD"})
row(363.22, 160.02, [("C", "C50", "1u", "+5V", "GND"), ("C", "C51", "1u", "TVDD", "GND"),
                     ("R", "R60", "56k", "TVDD", "VT_FB"), ("R", "R61", "10k", "VT_FB", "GND"),
                     ("R", "R62", "15k", "VT_FB", "VT_DAC"), ("R", "R63", "10k", "VT_PWM", "VT_DAC")])
row(363.22, 187.96, [("C", "C52", "1u", "VT_DAC", "GND"), ("R", "R64", "100k", "VT_EN", "GND"),
                     ("R", "R66", "68k", "VT_ADC", "GND"), ("C", "C53", "100n", "VT_ADC", "GND"),
                     ("R", "R65", "100k", "TVDD", "VT_ADC")])

# --- Target connector -----------------------------------------------------
notes.append((20, 270, "Connecteur cible, brochage PICkit 4/5"))
notes.append((20, 275, "(à recouper avec le guide utilisateur)"))
part("J2", "Connector_Generic:Conn_01x08", "ICSP", "Connector_PinSocket_2.54mm:PinSocket_1x08_P2.54mm_Horizontal",
     (116.84, 271.78), {"1": "ICSP_MCLR", "2": "TVDD", "3": "GND", "4": "ICSP_PGD", "5": "ICSP_PGC",
                      "6": "ICSP_AUX", "7": "ICSP_TDI", "8": "ICSP_TMS"})

# --- Power flags ----------------------------------------------------------
for i, net in enumerate(["GND", "+5V", "VBUS"]):
    part(f"#FLG{i + 1}", "power:PWR_FLAG", "PWR_FLAG", "", (134.62 + 7.62 * i, 76.2), {"1": net})


# --- JLCPCB parts (checked against the JLCPCB catalog, 2026-10-15) ---------
LCSC = {
    "100": "C22775", "100k": "C25803", "100n": "C14663", "10k": "C25804", "10u": "C15850",
    "15k": "C22809", "15p": "C1644", "1k": "C21190", "1u": "C15849", "200k": "C25811",
    "220": "C22962", "22k": "C31850", "22u 10V": "C45783", "22u 25V": "C12891",
    "22uH": "C354592", "27": "C25190", "330": "C23138", "47k": "C25819", "5.1k": "C23186",
    "500mA": "C720075", "56k": "C23206", "68k": "C23231", "82k": "C23254",
    "BAT54WS": "C7502694", "BOOTSEL": "C221896", "RESET": "C221896", "ICSP": "C22436149",
    "SWD": "C22438100", "SN74LVC1T45DBV": "C7843", "TLV75801PDBV": "C2877852",
    "W25Q16JVSSIQ": "C131025", "green": "C12624", "red": "C2286", "yellow": "C2287",
}
for _p in parts:
    if not _p["lcsc"] and not _p["ref"].startswith(("#", "TP")):
        _p["lcsc"] = LCSC[_p["value"]]


# ===========================================================================
def uid(*k):
    return str(uuid.uuid5(NS, "/".join(map(str, k))))


def prop(name, value, at, hide=False):
    eff = [Sym("effects"), [Sym("font"), [Sym("size"), 1.27, 1.27]]]
    if hide:
        eff.append([Sym("hide"), Sym("yes")])
    return [Sym("property"), name, value, [Sym("at"), at[0], at[1], 0], eff]


def geometry_check(segments):
    """Every pin point and stub must touch only its own pin: no stub end or
    segment may land on another pin or stub."""
    pts = {}
    for (a, b, owner) in segments:
        for q in (a, b):
            pts.setdefault(q, set()).add(owner)
    bad = [(q, o) for q, o in pts.items() if len(o) > 1]
    for (a, b, owner) in segments:
        for q, owners in pts.items():
            if owner in owners or q in (a, b):
                continue
            on_x = a[0] == b[0] == q[0] and min(a[1], b[1]) < q[1] < max(a[1], b[1])
            on_y = a[1] == b[1] == q[1] and min(a[0], b[0]) < q[0] < max(a[0], b[0])
            if on_x or on_y:
                bad.append((q, owners | {owner}))
    if bad:
        sys.exit("collisions: " + "; ".join(f"{q} {sorted(o)}" for q, o in bad[:10]))


def lib_prop(sym, name, value, X, Y, hide):
    q = copy.deepcopy(next(e for e in kisym.find(sym, "property") if e[1] == name))
    q[2] = value
    at = kisym.first(q, "at")
    at[1], at[2] = round(X + float(at[1]), 2), round(Y - float(at[2]), 2)
    eff = kisym.first(q, "effects")
    if eff is not None:
        eff[:] = [e for e in eff if not (isinstance(e, list) and e[0] == "hide")]
        if hide:
            eff.append([Sym("hide"), Sym("yes")])
    return q


def build():
    libs, items = {}, []
    used_pts = {}
    segments = []
    for p in parts:
        s = kisym.symbol(p["lib"])
        libs[p["lib"]] = s
        X, Y = p["at"]
        fp = p["fp"]
        if fp is None:
            fp = next(q[2] for q in kisym.find(s, "property") if q[1] == "Footprint")
        p["fp_resolved"] = fp
        pins = kisym.pins(s)
        names = {n for n, *_ in pins}
        missing = set(p["nets"]) - names
        assert not missing, (p["ref"], missing)
        is_power = p["lib"].startswith("power:")
        no_bom = is_power or p["lib"] == "Connector:TestPoint"
        props = [lib_prop(s, "Reference", p["ref"], X, Y, is_power),
                 lib_prop(s, "Value", p["value"], X, Y, is_power or p["lib"] == "Connector:TestPoint"),
                 prop("Footprint", fp, (X, Y), True), prop("Datasheet", "", (X, Y), True)]
        if p["lcsc"]:
            props.append(prop("LCSC", p["lcsc"], (X, Y), True))
        sym = [Sym("symbol"), [Sym("lib_id"), p["lib"]], [Sym("at"), X, Y, 0], [Sym("unit"), 1],
               [Sym("exclude_from_sim"), Sym("no")], [Sym("in_bom"), Sym("no" if no_bom else "yes")],
               [Sym("on_board"), Sym("no" if is_power else "yes")], [Sym("dnp"), Sym("no")],
               [Sym("uuid"), uid(p["ref"])], *props]
        sym += [[Sym("pin"), n, [Sym("uuid"), uid(p["ref"], "pin", n)]] for n in sorted(names)]
        sym.append([Sym("instances"), [Sym("project"), "picflash-board",
                                       [Sym("path"), "/" + ROOT, [Sym("reference"), p["ref"]], [Sym("unit"), 1]]]])
        items.append(sym)
        done = {}
        for n, name, typ, px, py, ang, unit in pins:
            sx, sy = round(X + px, 2), round(Y - py, 2)
            if n not in p["nets"]:
                if typ == "no_connect" or (sx, sy) in done:
                    continue
                sys.exit(f"{p['ref']} pin {n} ({name}) not assigned")
            net = p["nets"][n]
            if (sx, sy) in done:
                assert done[(sx, sy)] == net, (p["ref"], n, net, done[(sx, sy)])
                continue
            done[(sx, sy)] = net
            segments.append(((sx, sy), (sx, sy), (p["ref"], n)))
            if net is None:
                items.append([Sym("no_connect"), [Sym("at"), sx, sy], [Sym("uuid"), uid(p["ref"], "nc", n)]])
                continue
            out = {0: (-1, 0), 90: (0, 1), 180: (1, 0), 270: (0, -1)}[int(ang) % 360]
            ex, ey = round(sx + out[0] * STUB, 2), round(sy + out[1] * STUB, 2)
            assert (ex, ey) not in used_pts, (p["ref"], n, "stub collision", used_pts.get((ex, ey)))
            used_pts[(ex, ey)] = (p["ref"], n)
            segments[-1] = ((sx, sy), (ex, ey), (p["ref"], n))
            items.append([Sym("wire"), [Sym("pts"), [Sym("xy"), sx, sy], [Sym("xy"), ex, ey]],
                          [Sym("stroke"), [Sym("width"), 0], [Sym("type"), Sym("default")]],
                          [Sym("uuid"), uid(p["ref"], "w", n)]])
            angle, just = {(-1, 0): (180, "right"), (1, 0): (0, "left"), (0, -1): (90, "left"),
                           (0, 1): (270, "right")}[out]
            items.append([Sym("label"), net, [Sym("at"), ex, ey, angle],
                          [Sym("effects"), [Sym("font"), [Sym("size"), 1.27, 1.27]],
                           [Sym("justify"), Sym(just), Sym("bottom")]],
                          [Sym("uuid"), uid(p["ref"], "l", n)]])
    geometry_check(segments)
    for i, (x, y, t) in enumerate(notes):
        items.append([Sym("text"), t, [Sym("exclude_from_sim"), Sym("no")], [Sym("at"), x, y, 0],
                      [Sym("effects"), [Sym("font"), [Sym("size"), 2, 2], [Sym("bold"), Sym("yes")]],
                       [Sym("justify"), Sym("left"), Sym("bottom")]], [Sym("uuid"), uid("note", i)]])
    doc = [Sym("kicad_sch"), [Sym("version"), 20250114], [Sym("generator"), "picflash_gen"],
           [Sym("generator_version"), "9.0"], [Sym("uuid"), ROOT], [Sym("paper"), "A3"],
           [Sym("title_block"), [Sym("title"), "picflash : programmeur PIC (RP2040)"],
            [Sym("rev"), "0.1"], [Sym("comment"), 1, "Généré par hardware/tools/gen_schematic.py"],
            [Sym("comment"), 2, "Non testé : prototype, à relire avant fabrication"]],
           [Sym("lib_symbols"), *libs.values()], *items,
           [Sym("sheet_instances"), [Sym("path"), "/", [Sym("page"), "1"]]]]
    OUT.write_text(kisym.dump(doc) + "\n")
    (OUT.parent / "netlist.json").write_text(json.dumps(
        {p["ref"]: {k: v for k, v in p["nets"].items() if v} for p in parts if not p["ref"].startswith("#")},
        indent=1, sort_keys=True))
    (OUT.parent / "parts.json").write_text(json.dumps(
        {p["ref"]: dict(fp=p["fp_resolved"], value=p["value"], lcsc=p["lcsc"], uuid=uid(p["ref"]),
                        nets={k: v for k, v in p["nets"].items() if v})
         for p in parts if not p["ref"].startswith("#")}, indent=1, sort_keys=True))
    print(f"{OUT.name}: {len(parts)} symboles")


build()
