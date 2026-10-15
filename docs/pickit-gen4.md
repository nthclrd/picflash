# PICkit 4 / 5 / Basic : interpréteur Gen4 (mode MPLAB)

Faits d'interopérabilité établis par analyse statique des firmwares livrés avec
MPLAB X (PICkit4_TP 3.0.2719, PICkit5_TP 3.0.1195, PICkitBasic_TP 2.0.459) avec
REA + Ghidra, recoupés avec la pile Python Apache-2.0 des device packs et
avrdude `pickit5.c`. Aucun code ni octet de firmware n'est repris ici.
Statut : *observé* (lu dans le code) sauf mention *déduit*. Rien n'est encore
mesuré sur matériel.

## Messages USB

- En-tête 16 octets LE `[type][0][longueur totale][longueur de transfert]`,
  puis `[longueur params][longueur script][params][script]`.
- Message > 4 KiB : erreur 0x500 ; longueurs incohérentes : 0x503.
- Réponse (type 0x100) : `0x0D` à l'offset 0, code d'erreur à 16, longueur des
  résultats à 20, résultats à partir de 24 (512 octets max, sinon 0x502).
- Script avec upload (0x80000102) : la première réponse arrive *avant*
  l'exécution (erreurs de format seulement) ; les erreurs d'exécution sont dans
  la réponse au SCRIPT_DONE (0x103). Un script en échec s'arrête et envoie
  moins de données.

## Opcodes utilisés par picflash

| Op | Opérandes | Effet |
|----|-----------|-------|
| 0xEC | u32 | période d'horloge ICSP complète en **ns** ; défaut = la plus rapide (~100 ns) ; persiste entre scripts |
| 0x40 | 3 × u32 mV | mémorise VDD, VPP, VPP_op (aucun effet matériel immédiat) |
| 0x41 | — | niveau MCLR = pompe VPP, régulée 7,7–13,5 V ; si VPP < 7,6 V la pompe ne démarre pas ; attend 20 ms ; échec : 0x308/0x309 |
| 0x42 | — | niveau MCLR = rail I/O (VDD) ; coupe la pompe |
| 0x43 | — | source outil : VDD 1,5–5 V (bornée, 0x300/0x301) ; source externe : rail I/O = VDD mesuré, erreur 0x305 sous 0,6 V |
| 0x44 | — | VDD coupé, MCLR relâché (flottant, *déduit*) |
| 0x46 | u32 | source d'alimentation : 0 = externe, sinon outil ; avec 0, l'outil ne pilote jamais VDD |
| 0x47 | — | 40 octets : mV, VDD cible à l'offset 28 de la réponse |
| 0xB0 / 0xB1 | — | MCLR au niveau choisi / MCLR tiré à 0 |
| 0xB2 | u8 | bit 0 : CLK en entrée, bit 1 : DATA en entrée, bit 2 : CLK haut, bit 3 : DATA haut |
| 0xB5 / 0x83 | u8 nb bits, u32 | écrit LSb / MSb en premier, laisse les lignes basses |
| 0x80 | — | 16 coups d'horloge LSb first, mot de 14 bits |
| 0x84 | — | 24 coups d'horloge MSb first, charge utile de 16 bits non masquée (0xFFFF = DATA flottante) |
| 0x95 | — | les lectures poussent leurs 2 octets (remis à zéro à chaque message) |
| 0x94 / 0xA0 | u16 | attente en ms / µs |

## PICkit Basic

0x41 n'est qu'une attente, 0x42/0x43 ne font rien, 0x47 renvoie des constantes
(3,3 V) : pas de haute tension, pas de mesure de VDD, pas d'alimentation.

## Inconnu

Source analogique exacte de MCLR quand la pompe est coupée, sens des autres
mesures de 0x47, firmware du Snap, timings réels.
