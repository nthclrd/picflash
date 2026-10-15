# Carte picflash (RP2040)

Schéma KiCad 10 généré par `tools/gen_schematic.py` : la netlist Python est la
source de vérité, le `.kicad_sch` se régénère. Design original à partir des
datasheets publiques (RP2040 Hardware Design Guide, MT3608, TLV758,
SN74LVC1T45). **Non testé, à relire avant fabrication.**

```sh
cd hardware/tools
python3 gen_schematic.py     # écrit picflash-board.kicad_sch + netlist.json
python3 check_netlist.py     # netlist exportée par KiCad == spec
K=/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli
$K sch erc picflash-board.kicad_sch            # 0 violation attendue
$K sch export pdf -o build/picflash-board.pdf picflash-board.kicad_sch
```

Le générateur refuse toute collision géométrique (fil ou broche qui en touche
une autre), et `check_netlist.py` vérifie que chaque broche est sur le bon net.

## Blocs

| Bloc | Composants | Rôle |
|------|------------|------|
| USB | J1 USB-C, U2 USBLC6, F1 500 mA, R1/R2 5,1k | alimentation 5 V, données USB protégées |
| 3,3 V | U3 AP2112K | logique |
| MCU | U1 RP2040, U4 flash 16 Mbit, Y1 12 MHz, SW1 BOOTSEL, SW2 RESET, J3 SWD | |
| Lignes ICSP | U5-U9 SN74LVC1T45 | 1,65-5,5 V côté cible ; DIR tiré à 0 = cible en haute impédance au démarrage |
| VPP | U11 MT3608, 6-14 V | réglé par PWM filtré injecté dans FB, mesuré sur ADC1 |
| MCLR | Q1 (VPP via R53 220R), U10 + D5 (niveau VDD, LVP), Q3 (masse) | aucun actif = MCLR relâché |
| VDD cible | U12 TLV75801, 1,5-5 V | réglé par PWM sur FB, EN tiré à 0, mesuré sur ADC0 |
| Connecteur | J2 8 broches | brochage PICkit 4/5 : 1 MCLR, 2 VDD, 3 GND, 4 PGD, 5 PGC, 6 AUX, 7 TDI, 8 TMS |

## Brochage RP2040 (à reporter dans le firmware)

| GPIO | Signal | GPIO | Signal |
|------|--------|------|--------|
| 0/1 | DBG_TX / DBG_RX (TP1/TP2) | 13 | VPP_ON (Q1, actif haut) |
| 2/3 | PGC / PGC_DIR | 14 | MCLR_PD (Q3, actif haut) |
| 4/5 | PGD / PGD_DIR | 15 | BOOST_EN |
| 6/7 | AUX / AUX_DIR | 16 | VPP_PWM (PWM0 A) |
| 8/9 | TDI / TDI_DIR | 17 | VT_EN |
| 10/11 | TMS / TMS_DIR | 18 | VT_PWM (PWM1 A) |
| 12 | MCLR_LV (niveau VDD via U10/D5) | 20/21 | LED_ACT / LED_ERR |
| 26 | ADC0 VT_ADC = TVDD × 68/168 | 27 | ADC1 VPP_SENSE = VPP × 22/122 |
| 28 | ADC2 VBUS_SENSE = 5 V × 1/2 | | |

DIR = 1 : RP2040 vers la cible ; DIR = 0 : la cible est lue (et haute impédance).

## Règles pour le firmware

- Ne jamais activer VPP_ON et MCLR_PD en même temps (R53 limite à ~60 mA).
- Régler VT_PWM avant VT_EN ; vérifier sur ADC0 qu'aucune alimentation externe
  n'est présente avant d'activer VT_EN.
- Régler VPP en boucle fermée sur ADC1 avant VPP_ON ; PWM à 0 = tension max.

## Points ouverts

- Brochage J2 à recouper avec le guide utilisateur PICkit 4.
- Références LCSC renseignées seulement quand elles sont connues ; les autres
  (passifs, TLV75801, SN74LVC1T45, W25Q16JVSSIQ, LED, boutons, L1) sont à
  choisir dans le catalogue JLCPCB.
- Valeurs à valider : quartz (charge 15 pF), self 22 µH, stabilité du TLV758 et
  du MT3608 avec l'injection PWM dans FB.
- Le firmware actuel cible un Pi Pico (GP2/GP3/GP4) : il faut le porter sur ce
  brochage et ajouter le pilotage VPP/VDD.
- PCB (placement, routage, DRC, Gerber) pas encore fait.
