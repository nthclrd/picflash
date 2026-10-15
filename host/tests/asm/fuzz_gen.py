"""Random pic-as programs for differential testing. Usage: fuzz_gen.py OUTDIR N SEED"""
import random, sys, os
out, n, seed = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
PARTS = [("16F1789", 32), ("16F1509", 32), ("16F18877", 64), ("16F18446", 64)]
FILE_OPS = "addwf addwfc andwf asrf lslf lsrf comf decf decfsz incf incfsz iorwf movf rlf rrf subwf subwfb swapf xorwf".split()
LIT_OPS = "movlw retlw iorlw andlw xorlw sublw addlw".split()
PLAIN = "nop return retfie callw brw clrw clrwdt sleep reset".split()
REGS = ["PORTA", "TRISA", "LATA", "STATUS", "WREG", "BSR", "FSR0L", "FSR1H", "INTCON", "PCLATH"]

def num(r, v):
    f = r.choice(["dec", "hex0x", "hexh", "bin", "oct"])
    if v < 0: return str(v)
    if f == "dec": return str(v)
    if f == "hex0x": return hex(v)
    if f == "hexh": return "0%Xh" % v
    if f == "bin": return "0b" + bin(v)[2:]
    return "%oo" % v

def expr(r, consts, depth=0):
    if depth > 2 or r.random() < 0.4:
        if consts and r.random() < 0.4: return r.choice(consts)
        return num(r, r.randint(0, 300))
    a, b = expr(r, consts, depth + 1), expr(r, consts, depth + 1)
    op = r.choice(["+", "-", "*", "/", "mod", "shl", "shr", "and", "&", "|", "^", "<<", ">>", "high", "low", "~", "not"])
    if op in ("high", "low"): return "%s(%s)" % (op, a)
    if op in ("~", "not"): return "(%s %s)" % (op, a)
    if op in ("/", "mod"): b = num(r, r.randint(1, 20))
    if op in ("shl", "shr", "<<", ">>"): b = str(r.randint(0, 7))
    return "(%s %s %s)" % (a, op, b)

for i in range(n):
    r = random.Random(seed * 100000 + i)
    part, banks = r.choice(PARTS)
    L = ["PROCESSOR %s" % part, "#include <xc.inc>"]
    consts = []
    for c in range(r.randint(0, 4)):
        name = "K%d" % c
        L.append("%s EQU %s" % (name, expr(r, consts)))
        consts.append(name)
    if r.random() < 0.5:
        L += ["S0 SET %s" % expr(r, consts), "S0 SET S0 + 1"]; consts.append("S0")
    vars_ = []
    for ps in r.sample(["udata_shr", "udata_bank0", "udata_bank1", "udata_bank2", "udata"], r.randint(1, 4)):
        L.append("PSECT %s" % ps)
        for v in range(r.randint(1, 3)):
            name = "v_%s_%d" % (ps, v); vars_.append(name)
            L.append("%s: DS %d" % (name, r.randint(1, 4)))
    isr = r.random() < 0.6
    L += ["PSECT reset_vec, class = CODE, delta = 2", "reset_vec:"]
    if r.random() < 0.5: L.append(" pagesel start")
    L.append(" goto start")
    if isr: L += ["PSECT isr_vec, class = CODE, delta = 2", " retfie"]
    psects = ["code"] + ["extra%d" % k for k in range(r.randint(0, 2))]
    labels = []
    for pi, ps in enumerate(psects):
        if ps == "code": L.append("PSECT code")
        else: L.append("PSECT %s, class = CODE, delta = 2%s" % (ps, r.choice(["", ", reloc = 4"])))
        local = []
        if ps == "code": L.append("start:"); local.append("start")
        for k in range(r.randint(3, 25)):
            if r.random() < 0.15:
                lab = "%s_l%d" % (ps, k); L.append("%s:" % lab); local.append(lab); labels.append(lab)
            t = r.random()
            regs = REGS + vars_
            if t < 0.25:
                op = r.choice(FILE_OPS); f = r.choice(regs + [num(r, r.randint(0, 0x7F))])
                d = r.choice(["", ", f", ", w", ", 0", ", 1", ", F", ", W"])
                L.append(" %s %s%s" % (op, f, d))
            elif t < 0.4:
                L.append(" %s %s" % (r.choice(LIT_OPS), expr(r, consts)))
            elif t < 0.5:
                L.append(" %s %s, %d" % (r.choice(["bcf", "bsf", "btfsc", "btfss"]), r.choice(regs), r.randint(0, 7)))
            elif t < 0.55:
                L.append(" banksel %s" % r.choice(regs))
            elif t < 0.6:
                L.append(" movlb %d" % r.randint(0, banks - 1))
            elif t < 0.66 and local:
                L.append(" bra %s" % r.choice(local))
            elif t < 0.72:
                L.append(" %s %s" % (r.choice(["goto", "call"]), r.choice(["start"] + labels)))
            elif t < 0.75:
                L.append(" pagesel %s" % r.choice(["start"] + labels))
            elif t < 0.8:
                L.append(" %s" % r.choice(PLAIN))
            elif t < 0.84:
                L.append(" addfsr %s, %d" % (r.choice(["FSR0", "FSR1"]), r.randint(-32, 31)))
            elif t < 0.9:
                fsr = r.choice(["FSR0", "FSR1"])
                form = r.choice(["++%s", "--%s", "%s++", "%s--", "%d[%%s]" % r.randint(-32, 31)]) % fsr
                L.append(" %s %s" % (r.choice(["moviw", "movwi"]), form))
            elif t < 0.94:
                L.append(" movwf %s" % r.choice(regs))
            elif t < 0.97:
                L.append(" clrf %s" % r.choice(regs))
            else:
                L.append(" dw %s" % ", ".join(expr(r, consts) for _ in range(r.randint(1, 3))))
        if ps != "code" and r.random() < 0.5:
            L.append(" movlp %d" % r.randint(0, 127))
    L.append(" end reset_vec")
    os.makedirs(out, exist_ok=True)
    open(os.path.join(out, "p%04d.asm" % i), "w").write("\n".join(L) + "\n")
