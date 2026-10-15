; 64-bank part with 8-bit ICSP: wide MOVLB, five config words.
PROCESSOR 16F18446
#include <xc.inc>

  CONFIG  FEXTOSC = OFF, RSTOSC = HFINT1
  CONFIG  MCLRE = ON, WDTE = OFF, LVP = ON

PSECT udata_bank0
count:  DS 1

PSECT reset_vec, class = CODE, delta = 2
reset_vec:
        goto    start

PSECT code
start:
        banksel TRISA
        clrf    BANKMASK(TRISA)
        banksel ANSELA
        clrf    BANKMASK(ANSELA)
        banksel count
loop:   banksel LATA
        bsf     LATA, 2
        banksel count
        decfsz  BANKMASK(count), f
        goto    loop
        movlb   63
        goto    loop
        end     reset_vec
