# picflash

Programmeur PIC open source en Rust : l'outil `picflash` remplace MPLAB IPE et
pilote au choix une carte maison à RP2040 (proto sur Pi Pico en attendant le
PCB dédié) ou les outils Microchip : PICkit 4, PICkit 5, PICkit Basic, MPLAB
Snap.

Algorithmes écrits depuis les *Memory Programming Specifications* publiques
(PIC16(L)F178X, PIC16(L)F184XX DS40001970), recoupés avec les séquences des
scripts PICkit 4 (voir `docs/pickit-gen4.md`). Les paramètres de chaque
part (mémoire, IDs, config, registres, bits) viennent d'une base embarquée
(`host/data/devices.json.gz`) extraite des device packs Microchip, sous licence
Apache-2.0 (`host/data/LICENSE-microchip-dfp.txt`). Aucun code, script ni
firmware Microchip n'est copié dans ce repo.

**Ni MPLAB X ni XC8 ne sont nécessaires** : picflash assemble, programme et
désassemble seul (vérifié en interdisant l'accès à `/Applications/microchip`).

## Programmateurs

| Outil            | Entrée basse tension (LVP) | Haute tension (VPP) | Alimente la cible |
|------------------|:--:|:--:|:--:|
| carte picflash (PCB `hardware/`) | oui | oui (défaut) | oui, 1,5-4,8 V (`--power 3.3`, `-W3.3`) |
| prototype Pi Pico | oui | non | non |
| PICkit 4 / 5     | oui | oui (défaut, comme IPE) | oui (`--power 5`, `-W5`) |
| PICkit Basic     | oui | non | non |
| MPLAB Snap       | oui | non | non |

Les outils Microchip doivent être en **mode MPLAB** (celui qu'utilise MPLAB X ;
les modes AVR / CMSIS-DAP sont détectés et signalés). picflash leur envoie ses
propres scripts « Gen4 », traduits depuis les mêmes séquences bit à bit que la
carte picflash (`host/src/pickit.rs`). Sources, sans code Microchip copié : les
opcodes et formats d'opérandes Gen4 viennent de la pile Python des device packs
(`scripts/*/common`, Apache-2.0), le cadrage USB et les commandes
d'alimentation du pilote `pickit5.c` d'avrdude (faits de protocole seulement).

```sh
picflash write blink.hex                 # auto : carte picflash, sinon PICkit/Snap
picflash --tool pickit --lv write blink.hex
picflash --tool pickit --power 3.3 id    # PICkit 4/5 alimente la cible en 3,3 V
picflash-ipecmd -TPPK4 -E -M -P16F1789 -Fblink.hex -OL [-L] [-W5]
```

Non testé sur matériel : la traduction Gen4 est vérifiée contre le PIC simulé
(`host/tests/pickit_flows.rs`, mutations incluses), pas encore sur un vrai
PICkit. Sous Windows, le PICkit doit utiliser le pilote WinUSB installé par
MPLAB X ; sous Linux, la règle udev du `.deb` donne l'accès.

## Parts supportées

```sh
picflash parts            # résumé par spec de programmation
picflash parts --all      # liste complète
picflash -P PIC16F18446 info
```

201 parts sur 21 specs, deux familles :

| Famille        | Commandes                         | Exemples                    |
|----------------|-----------------------------------|-----------------------------|
| `Midrange6Bit` | 6 bits LSb first, clé LSb + 1 clk | PIC16F1789, PIC16F1829…     |
| `Midrange8Bit` | 8 bits + payload 24 bits MSb first | PIC16F18446, PIC16F18877…   |

Une spec n'est déclarée que si *toutes* ses parts utilisent exactement les mêmes
séquences (entrée, ID, effacement, écriture ligne/config, lecture) que la
référence de la famille. Exclues pour l'instant : contournement d'errata à
l'effacement (DS40001390, 40001397, 40001654), autre jeu de commandes
(DS40001738), Bulk Erase avec payload de régions (DS40002266, 2276, 2317, 2500,
2540, 2646). Ajouter une spec = une ligne dans `SPECS` (`host/src/device.rs`)
une fois sa séquence vérifiée.

## Installer (sans ligne de commande)

| Système | Fichier                    | Remarque                                                      |
|---------|----------------------------|---------------------------------------------------------------|
| macOS   | `picflash_x.y.z_*.dmg`     | glisser dans Applications ; 1er lancement : clic droit > Ouvrir (app non signée) |
| Windows | `picflash_x.y.z_*-setup.exe` | « Informations complémentaires > Exécuter quand même » (SmartScreen) |
| Linux   | `.deb` ou `.AppImage`      | le `.deb` installe la règle udev de la carte                  |

Aucun prérequis. Utilisation : déposer un `.asm` dans la fenêtre (assemblé par
picflash, erreurs surlignées dans la vue Source), puis « Programmer » ; ou
déposer un `.hex`.

Construire les installeurs : `cd gui && cargo packager --release --formats app,dmg`
en local (macOS), ou le workflow `installeurs` (tag `v*`) pour les 4 plateformes.

## Outils

| Binaire           | Rôle                                                        |
|-------------------|-------------------------------------------------------------|
| `picflash`        | CLI : `asm`, `info`, `parts`, `disasm`, `id`, `write`, `read`… |
| `picflash-ipecmd` | remplaçant d'`ipecmd`, mêmes options (`-TP -P -F -M -E -Y -OL -L -W`) |
| `picflash-gui`    | fenêtre : .asm → compile → programme, vues source/hex/désassemblée colorées |

`-TPSIM` (ipecmd) ou « Simulateur » (GUI) exécutent tout sur un PIC simulé bit
à bit : utile sans carte.

```sh
cargo install --path host      # picflash + picflash-ipecmd dans ~/.cargo/bin
picflash asm blink.asm         # -> blink.hex, sans XC8
picflash -P16F1789 disasm blink.hex
picflash-ipecmd -TPSIM -E -M -P16F1789 -F"blink.hex" -OL
cargo run --release -p picflash-gui -- blink.hex [--asm]
```

## Fonctions IPE

| Fonction | CLI | ipecmd | Interface |
|---|---|---|---|
| EEPROM : écriture, vérification, lecture | `write`, `read` | `-M`, `-ME`, `-YE` | vue « EEPROM » |
| Préserver l'EEPROM à l'effacement | `--keep-eeprom` | `-Z` (`-E` l'emporte) | « Préserver l'EEPROM » |
| Programmation partielle | `--only program,eeprom,ids,config` | `-MP -ME -MI -MC` | menu « Zones » |
| Checksum (comme IPE) | `checksum x.hex` | `-K` | à côté du fichier HEX |
| Bits de configuration | | | « Config… » puis « Enregistrer… » |
| Enregistrer un .hex | `read out.hex` | `-GF` | « Enregistrer… » (fichier ou puce lue) |

Programmation partielle : les lignes de mémoire programme sont effacées une à
une (commande Row Erase) ; la config ne peut que passer des bits de 1 à 0 sans
effacement complet, ce qui est vérifié avant d'écrire. Le checksum suit la
formule des specs (DS40001970 §2.4.3) et retrouve leur tableau d'exemples.

## Débogueur (simulé)

Onglet « Débogueur » : le programme tourne sur un simulateur du cœur PIC16
enhanced (`host/src/emu.rs`, 49 instructions, pile, banques, FSR, cycles) :
Continuer (F5), Pas (F11), Pas par-dessus (F10), Sortir, Reset, points d'arrêt
dans la marge, registres du cœur, pile, LED des ports, RAM par banque, en temps
réel à la fréquence choisie. Les périphériques (timers, ADC, interruptions)
ne sont pas simulés. Le débogage sur la vraie puce via un PICkit n'est pas
encore fait.

## Assembleur

`picflash asm` accepte les sources écrites pour `pic-as` (XC8) sur PIC16
enhanced mid-range : `PROCESSOR`, `CONFIG CHAMP = VALEUR`, `#include <xc.inc>`,
`#define`/`#if`, `PSECT` (class, delta, reloc, abs), `DS`/`DB`/`DW`,
`EQU`/`SET`, `ORG`/`ALIGN`, `BANKSEL`/`PAGESEL`, `MACRO`/`LOCAL`/`REPT`,
`IF`/`ELSE`, les 49 instructions et les opérateurs de `pic-as`. `reset_vec` et
`isr_vec` sont placés en 0 et 4 comme dans le Makefile du labo ; `--link
-Wl,-pnom=adresse` place d'autres psects.

Vérifié par comparaison avec `pic-as` (XC8 v3.10), boîte noire :

- 4 programmes écrits à la main (dont le `blink.asm` du labo, `.hex` identique
  octet par octet) ;
- 400 programmes aléatoires (`host/tests/asm/fuzz_gen.py`) sur PIC16F1789,
  1509, 18446 et 18877 : images mémoire identiques, et les sources que
  `pic-as` refuse sont refusées aux mêmes lignes. 30 sont gardés en test.

Seule différence connue : un `DW` de plus de 14 bits est tronqué à 14 bits
(ce que garde la puce) au lieu d'être recopié tel quel dans le `.hex`.

Non supporté : PIC18, PIC10/12 baseline, plusieurs fichiers liés ensemble,
`IRP`, labels temporaires (`1b`/`1f`), listing `.lst`.

## Désassembleur

Le désassembleur couvre le cœur PIC16 enhanced (49 instructions, variantes 32
et 64 banques vérifiées contre pic-as) et nomme les registres d'après le DFP,
avec suivi de BSR par analyse de flot.

## Structure

| Dossier     | Rôle                                                                 |
|-------------|----------------------------------------------------------------------|
| `protocol/` | `no_std`, partagé : protocole USB v3 (ops bit à bit, tensions)       |
| `host/`     | CLI `picflash` : DFP, HEX, familles ICSP, erase/write/verify/read    |
| `firmware/` | RP2040 + embassy : USB CDC, MCLR 3 états, VPP/VDD régulés (carte)    |
| `hardware/` | carte KiCad 4 couches, fichiers JLCPCB (voir `hardware/README.md`)   |

L'hôte envoie des *scripts* (≤ 4 KiB) d'ops bas niveau : `ENTER`, `ENTER_HV`,
`RELEASE`, `OUT n bits (LSb/MSb first)`, `IN n bits`, `DELAY`, et sur la carte
`SET_VPP`, `SET_VDD`, `MEASURE`, `CAPS`. Le firmware règle chaque tension en
boucle fermée sur son ADC et coupe tout quand l'hôte se déconnecte ; picflash
choisit VPP d'après la puce (9 V pour PIC16F1xxx). Le firmware ne connaît
aucune famille de PIC : toute la logique est côté PC, une nouvelle famille ne
demande pas de reflasher la carte.

## Câblage (prototype Pi Pico)

| Pico            | PIC16F1789 (PDIP-40)    | Connecteur ICSP / PICkit |
|-----------------|-------------------------|--------------------------|
| GP4             | MCLR/VPP (broche 1)     | 1                        |
| 3V3(OUT) (36)   | VDD (11, 32)            | 2                        |
| GND (38)        | VSS (12, 31)            | 3                        |
| GP3             | ICSPDAT / RB7 (40)      | 4                        |
| GP2             | ICSPCLK / RB6 (39)      | 5                        |

- Cible alimentée en **3,3 V** par le Pico. Une cible en 5 V demande un
  level shifter (3,3 V < VIH des broches ICSP à 5 V).
- LVP doit rester actif (`CONFIG LVP = ON`). `picflash write` refuse une config
  qui coupe LVP, sauf `--allow-lvp-off` (il faudrait alors un programmeur HV).

## Utilisation

```sh
# Firmware : brancher la carte (ou le Pico) en maintenant BOOTSEL, puis copier
# sur le disque RPI-RP2 :
#   picflash-firmware.uf2       carte picflash
#   picflash-firmware-pico.uf2  prototype Pi Pico
cd firmware && cargo build --release                                    # carte
cd firmware && cargo build --release --no-default-features --features pico --target-dir target-pico

# Hôte
cargo install --path host
picflash ping
picflash id
picflash write blink.hex                  # erase + write + verify + run
picflash read dump.hex
```

Dans le `Makefile` du lab :

```make
PROGRAMMER ?= PPF                       # PSIM = simulateur
IPECMD ?= $(HOME)/.cargo/bin/picflash-ipecmd
flash: $(EXEC)
	$(IPECMD) $(FLASH_ARGS)             # FLASH_ARGS d'IPE inchangés
```

## Tests

```sh
cargo test          # DFP, hex, protocole, et flows contre des PIC simulés
```

Le simulateur décode chaque front d'horloge comme le silicium : clé LVP, ordre
des bits, start/stop, PC, latches, flash qui ne fait que passer des bits à 0,
fenêtres busy (commande pendant une écriture = erreur). Testé par mutation :
inverser l'ordre de la clé, oublier le 33e coup d'horloge, ne pas décaler le
payload, lire en LSb first, sauter une attente ou effacer depuis le mauvais PC
fait échouer les tests. Il ne remplace pas un test sur vraie puce.

## État

Testé uniquement sur un PIC simulé : ni le PICkit, ni la carte, ni le firmware
n'ont encore tourné sur du vrai matériel. La carte (`hardware/`) n'a pas été
fabriquée. Développé avec l'aide de Claude (Anthropic).

## Licence

MIT, voir `LICENSE`. Les données de puces embarquées (`host/data/`) viennent
des device packs Microchip, sous Apache-2.0 (`host/data/LICENSE-microchip-dfp.txt`).
