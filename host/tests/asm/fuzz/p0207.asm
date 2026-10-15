PROCESSOR 16F1509
#include <xc.inc>
K0 EQU 6
K1 EQU (K0 shl 2)
K2 EQU 0CFh
K3 EQU K2
S0 SET 0x91
S0 SET S0 + 1
PSECT udata_bank0
v_udata_bank0_0: DS 2
v_udata_bank0_1: DS 4
v_udata_bank0_2: DS 2
PSECT udata_shr
v_udata_shr_0: DS 1
PSECT udata
v_udata_0: DS 1
v_udata_1: DS 4
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 addfsr FSR1, -3
code_l1:
 clrf BSR
 xorlw (~ (117 | (041h & 122)))
 decf FSR1H, 0
 movlb 0
PSECT extra0, class = CODE, delta = 2
 addwf v_udata_0, f
 incfsz PCLATH, 0
extra0_l2:
 movf WREG, w
extra0_l3:
 addlw low(165o)
 moviw ++FSR0
 banksel FSR1H
 addwf v_udata_bank0_0
 return
extra0_l8:
 addwf v_udata_shr_0
 nop
 call start
 addlw 055h
 iorwf TRISA, 0
 movwi 1[FSR0]
 retlw K3
 bsf v_udata_bank0_1, 2
 movlp 9
PSECT extra1, class = CODE, delta = 2, reloc = 4
 call extra0_l8
 movwi FSR1++
extra1_l2:
 addfsr FSR1, -31
extra1_l3:
 goto start
extra1_l4:
 addwf PCLATH, f
 decfsz v_udata_shr_0
 bra extra1_l3
 movlp 108
 end reset_vec
