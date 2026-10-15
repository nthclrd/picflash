PROCESSOR 16F1789
#include <xc.inc>
K0 EQU 085h
K1 EQU K0
PSECT udata_bank0
v_udata_bank0_0: DS 2
PSECT udata_bank1
v_udata_bank1_0: DS 3
v_udata_bank1_1: DS 1
v_udata_bank1_2: DS 4
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 clrf TRISA
 bra start
 asrf v_udata_bank1_2, 0
 xorwf v_udata_bank1_1
 bsf TRISA, 1
 iorwf v_udata_bank1_2, 0
 iorlw (K0 and K0)
 clrf PORTA
 reset
code_l9:
 bcf FSR0L, 6
 addfsr FSR0, 16
 subwf STATUS
 bra start
 iorlw (K0 << 2)
 moviw FSR0--
 bra start
 pagesel start
 call code_l9
 moviw FSR0++
 movf FSR0L, 1
 movwf v_udata_bank1_1
 iorwf BSR, F
 clrf v_udata_bank0_0
 subwfb 033h
PSECT extra0, class = CODE, delta = 2, reloc = 4
extra0_l0:
 movlw ((K1 mod 0xa) << 1)
 moviw 13[FSR1]
 clrw
 banksel v_udata_bank0_0
 dw 360o, 51, (~ K0)
 call extra0_l0
extra0_l6:
 clrf WREG
 xorwf STATUS
 bcf v_udata_bank1_1, 1
 incfsz TRISA, 1
 btfsc PORTA, 0
 decf v_udata_bank1_1, 1
 btfss LATA, 3
 retlw ((K1 * (K0 shl 0)) ^ ((78 shl 0) * K0))
extra0_l14:
 andwf v_udata_bank1_2, 1
 bra extra0_l0
 incfsz v_udata_bank1_1
 movlw ((K1 << 1) - (K0 | 03Bh))
 banksel FSR0L
 sublw low(176)
extra0_l20:
 moviw ++FSR0
 bcf PORTA, 0
 dw (((0115h - 0B4h) mod 14) << 4)
 end reset_vec
