PROCESSOR 16F1789
#include <xc.inc>
S0 SET (((0x8d mod 03h) and 0x2e) >> 7)
S0 SET S0 + 1
PSECT udata_bank0
v_udata_bank0_0: DS 4
v_udata_bank0_1: DS 2
PSECT udata_shr
v_udata_shr_0: DS 4
v_udata_shr_1: DS 4
PSECT udata
v_udata_0: DS 2
v_udata_1: DS 3
v_udata_2: DS 2
PSECT udata_bank1
v_udata_bank1_0: DS 3
v_udata_bank1_1: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 btfss TRISA, 7
 decfsz v_udata_shr_0
 bra start
code_l3:
 movlb 10
 addwf v_udata_bank0_0, w
 moviw -8[FSR1]
code_l6:
 goto code_l6
 goto start
 brw
code_l9:
 movwf PCLATH
PSECT extra0, class = CODE, delta = 2, reloc = 4
 subwfb v_udata_0, 1
 banksel STATUS
 addwf v_udata_1, w
 andwf FSR1H, 1
 addfsr FSR1, -18
 bsf LATA, 5
 xorlw (((~ S0) shl 7) << 3)
 return
 movwi FSR0++
 clrw
 xorlw 0b11111001
 asrf LATA, 1
extra0_l12:
 dw high(0x58), (09Eh & ((S0 - 0x10b) shr 0)), 415o
 swapf v_udata_0, w
 addfsr FSR1, -25
 addwfc v_udata_1, w
 banksel BSR
extra0_l17:
 movlw (low(407o) shr 6)
 movwi -32[FSR0]
 movlp 67
PSECT extra1, class = CODE, delta = 2
extra1_l0:
 goto extra0_l12
 movf STATUS, w
 moviw FSR0--
 banksel v_udata_1
extra1_l4:
 addfsr FSR1, 8
 pagesel extra1_l4
 movwf BSR
 movlp 121
 end reset_vec
