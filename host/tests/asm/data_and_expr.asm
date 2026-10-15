; Psect placement, data allocation, expressions and directives.
PROCESSOR 16F1789
#include <xc.inc>

  CONFIG  FOSC = HS, WDTE = ON
  CONFIG  PWRTE = ON
  CONFIG  LVP = ON, PLLEN = OFF

#define LED_PORT PORTA
#define LED_BIT 3
#define TWICE(x) ((x) * 2)

COUNT   EQU     5
LIMIT   EQU     COUNT * 3 + 1
STEP    SET     2
STEP    SET     STEP + 1

PSECT udata_shr
flags:  DS      1
PSECT udata_bank0
buf0:   DS      4
var0:   DS      1
PSECT udata_bank1
buf1:   DS      2
PSECT udata
anyvar: DS      3

PSECT reset_vec, class = CODE, delta = 2
reset_vec:
        pagesel start
        goto    start

PSECT isr_vec, class = CODE, delta = 2
        retfie

PSECT tables, class = CODE, delta = 2
table:
        brw
        retlw   'A'
        retlw   'z'
        retlw   0b1010
        retlw   7Fh
        retlw   LIMIT
        retlw   STEP
        retlw   TWICE(3)
        retlw   high(table)
        retlw   low(table)
        dw      0x1234, 0x3FFF, LIMIT
        retlw   (1 << 4) | 3
        retlw   0x0F & ~1
        retlw   100 / 7
        retlw   100 mod 7
        retlw   -1 & 0xFF

PSECT code
start:
        banksel var0
        movwf   BANKMASK(var0)
        banksel buf1
        clrf    BANKMASK(buf1)
        incf    BANKMASK(buf1+1), f
        banksel anyvar
        movf    BANKMASK(anyvar), w
        bsf     flags, 1
        banksel LED_PORT
        bsf     BANKMASK(LED_PORT), LED_BIT
        bcf     RA2
        movlw   COUNT
        movlw   low(here)
here:   movlw   high($)
#ifdef LED_BIT
        movlw   1
#else
        movlw   2
#endif
#ifndef UNDEFINED_THING
        movlw   3
#endif
        call    table
        goto    $
        end     reset_vec
