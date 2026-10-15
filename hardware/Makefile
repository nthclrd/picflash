# Flux complet : make all FREEROUTING=/chemin/freerouting-2.4.1.jar
KICAD   ?= /Applications/KiCad/KiCad.app/Contents
KCLI    := $(KICAD)/MacOS/kicad-cli
KPY     := $(KICAD)/Frameworks/Python.framework/Versions/3.9/bin/python3
FREEROUTING ?= $(error passe FREEROUTING=/chemin/freerouting.jar)

.PHONY: all schematic pcb route finish check fab render

all: schematic pcb route finish check fab

schematic:
	cd tools && python3 gen_schematic.py && python3 check_netlist.py

pcb:            # placement + export DSN
	cd tools && $(KPY) gen_pcb.py

route:          # autoroutage des signaux (Freerouting) ; masse et 3V3 par les plans
	python3 tools/dsn_filter.py build/picflash-board.dsn
	cd build && java -jar $(FREEROUTING) -de picflash-board.dsn -do picflash-board.ses -mp 100 --gui.enabled=false -l en > route.log 2>&1
	grep "Optimization stage completed" build/route.log | sed 's/.*final score/score/'

finish:         # import du routage, plans de masse, remplissage
	cd tools && $(KPY) finish_pcb.py

check:
	$(KCLI) sch erc -o build/erc.rpt picflash-board.kicad_sch
	$(KCLI) pcb drc --schematic-parity --severity-all -o build/drc.rpt picflash-board.kicad_pcb
	@! grep -q "; error$$" build/drc.rpt || (echo "DRC : erreurs" && false)

fab:
	cd tools && python3 export_fab.py

render:
	$(KCLI) pcb render --side top --width 2000 --height 1300 --quality high -o build/render-top.png picflash-board.kicad_pcb
