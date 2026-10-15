# picflash

Programmeur PIC open source en Rust, pour remplacer MPLAB IPE. Il pilote une
carte maison à RP2040 (prototype sur Pi Pico en attendant le PCB) ou un PICkit
4, PICkit 5, PICkit Basic ou MPLAB Snap. Il assemble, programme et désassemble
sans MPLAB X ni XC8.

**État :** testé seulement sur un PIC simulé. Ni le PICkit, ni la carte, ni le
firmware n'ont tourné sur du vrai matériel ; la carte (`hardware/`) n'a pas été
fabriquée. Développé avec l'aide de Claude (Anthropic).

## Installer

| Système | Fichier | Remarque |
|---------|---------|----------|
| macOS   | `.dmg`  | 1er lancement : clic droit > Ouvrir (app non signée) |
| Windows | `-setup.exe` | SmartScreen : « Exécuter quand même » |
| Linux   | `.deb` ou `.AppImage` | le `.deb` installe la règle udev |

Dans la fenêtre : déposer un `.asm` ou un `.hex`, puis « Programmer ».
En ligne de commande : `cargo install --path host`.

## Utiliser

```sh
picflash asm blink.asm               # -> blink.hex
picflash write blink.hex             # efface, écrit, vérifie, lance
picflash read dump.hex
picflash -P16F1789 disasm blink.hex
picflash --tool pickit --power 3.3 id
picflash parts                       # puces supportées
picflash-ipecmd -TPSIM -E -M -P16F1789 -Fblink.hex -OL   # mêmes options qu'ipecmd
```

`-TPSIM` (ou « Simulateur » dans la fenêtre) fait tout sur un PIC simulé, sans
matériel. L'onglet « Débogueur » exécute le programme pas à pas sur ce
simulateur (sans périphériques).

Dans le `Makefile` du labo, remplacer `ipecmd` par `picflash-ipecmd` :

```make
IPECMD ?= $(HOME)/.cargo/bin/picflash-ipecmd
```

## Programmateurs

| Outil | Basse tension (LVP) | Haute tension (VPP) | Alimente la cible |
|-------|:--:|:--:|:--:|
| carte picflash | oui | oui | 1,5-4,8 V |
| prototype Pi Pico | oui | non | non |
| PICkit 4 / 5 | oui | oui | oui |
| PICkit Basic, Snap | oui | non | non |

Les outils Microchip doivent être en mode MPLAB. Windows : pilote WinUSB
installé par MPLAB X. Détails du protocole : `docs/pickit-gen4.md`.

## Puces supportées

201 PIC16 enhanced mid-range (familles PIC16F1xxx et PIC16F18xxx), décrites par
la base `host/data/devices.json.gz` extraite des device packs Microchip
(Apache-2.0). Une spec de programmation n'est ajoutée que si toutes ses puces
suivent exactement la même séquence (`SPECS` dans `host/src/device.rs`).

## Assembleur

`picflash asm` accepte les sources `pic-as` (XC8) pour PIC16 enhanced :
directives, macros, `PSECT`, `CONFIG`, `#include <xc.inc>`. Vérifié contre
`pic-as` v3.10 sur le `blink.asm` du labo et 400 programmes aléatoires : même
`.hex`, mêmes erreurs. Non supporté : PIC18, PIC10/12, plusieurs fichiers
liés, `IRP`, labels temporaires.

## Câblage du prototype Pi Pico

| Pico | PIC16F1789 (PDIP-40) | ICSP |
|------|----------------------|------|
| GP4 | MCLR (1) | 1 |
| 3V3(OUT) | VDD (11, 32) | 2 |
| GND | VSS (12, 31) | 3 |
| GP3 | ICSPDAT (40) | 4 |
| GP2 | ICSPCLK (39) | 5 |

Cible en 3,3 V. Garder `CONFIG LVP = ON` : `picflash write` refuse sinon.

Firmware : brancher en maintenant BOOTSEL, copier `picflash-firmware-pico.uf2`
(ou `picflash-firmware.uf2` pour la carte) sur le disque RPI-RP2.

## Structure

| Dossier | Rôle |
|---------|------|
| `host/` | CLI, assembleur, simulateur, programmation |
| `gui/` | interface |
| `protocol/` | protocole USB partagé hôte/firmware |
| `firmware/` | RP2040 : exécute les séquences bit à bit envoyées par l'hôte |
| `hardware/` | carte KiCad et fichiers JLCPCB |

Toute la logique PIC est côté PC : une nouvelle famille ne demande pas de
reflasher la carte.

## Tests

```sh
cargo test
```

Le PIC simulé décode chaque front d'horloge comme la vraie puce ; les tests
échouent si une séquence est fausse (ordre des bits, attente oubliée…). Ça ne
remplace pas un essai sur vraie puce.

## Licence

MIT (`LICENSE`). Données de puces (`host/data/`) : Apache-2.0, Microchip.
