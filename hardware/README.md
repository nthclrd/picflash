# Carte picflash (RP2040)

Design original tiré des datasheets publiques (RP2040, MT3608, TLV758,
SN74LVC1T45). **Non testé, à relire avant fabrication.**

## Générer

Le schéma et le PCB sont générés par les scripts de `tools/` : la netlist
Python (`tools/gen_schematic.py`) est la source de vérité.

```sh
make all FREEROUTING=/chemin/freerouting-2.4.1.jar
```

Étapes : schéma généré et vérifié net par net, placement, routage automatique
(Freerouting), plans de masse, ERC/DRC (0 erreur), puis `fab/` : Gerber,
perçages, BOM et CPL au format JLCPCB. Chaque pièce de la BOM a un numéro LCSC
(stock vérifié le 15/10/2026, à revérifier à la commande).

Carte 72 × 44 mm, 4 couches (In1 masse, In2 +3,3 V), composants sur une face.

## Blocs

| Bloc | Composants | Rôle |
|------|------------|------|
| USB | J1 USB-C, U2 USBLC6, F1 | alimentation 5 V, données protégées |
| 3,3 V | U3 AP2112K | logique |
| MCU | U1 RP2040, U4 flash, Y1 12 MHz | |
| Lignes ICSP | U5-U9 SN74LVC1T45 | adaptation 1,65-5,5 V, haute impédance au démarrage |
| VPP | U11 MT3608 | 6,7-13,9 V, réglé par PWM, mesuré sur ADC1 |
| VDD cible | U12 TLV75801 | 1,5-4,86 V, réglé par PWM, mesuré sur ADC0 |
| MCLR | Q1, U10 + D5, Q3 | VPP, niveau VDD ou masse |
| Connecteur | J2 8 broches | brochage PICkit 4/5 |

Brochage GPIO : `firmware/src/hw_board.rs`.

## Règles pour le firmware

- Ne jamais activer VPP_ON et MCLR_PD en même temps.
- PWM à 0 = tension max (13,9 V pour VPP) : fixer le PWM avant d'activer le
  régulateur.
- Avant d'alimenter la cible, vérifier sur ADC0 qu'aucune alimentation
  externe n'est présente.

## Avant de commander

- Relire le routage automatique : masse sous l'USB, boucle du MT3608, pistes
  VPP/MCLR.
- Vérifier les rotations du CPL dans l'aperçu JLCPCB.
- Valeurs à valider sur la première carte : self 22 µH, stabilité des
  régulateurs avec l'injection PWM.
- J2 est traversant : assemblage THT ou soudure à la main.
