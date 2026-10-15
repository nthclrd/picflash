PROCESSOR 16F18877
#include <xc.inc>
K0 EQU 0E8h
K1 EQU (~ K0)
PSECT udata_bank1
v_udata_bank1_0: DS 3
PSECT udata_bank2
v_udata_bank2_0: DS 2
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT code
start:
 movlb 26
 movlb 58
 bcf LATA, 1
 andwf FSR0L, 1
 movlb 40
 bsf WREG, 3
code_l6:
 addwfc TRISA, F
 clrf v_udata_bank2_0
 btfsc INTCON, 3
PSECT extra0, class = CODE, delta = 2, reloc = 4
 goto code_l6
extra0_l1:
 btfss PORTA, 3
 addlw (((0BBh + 0x95) shl 6) & K0)
 bcf FSR0L, 2
 andlw K0
extra0_l5:
 rrf PCLATH, W
 lsrf PORTA, 1
extra0_l7:
 bsf PORTA, 5
 retlw ((~ K1) and (0x47 + K0))
 bra extra0_l7
 xorwf v_udata_bank1_0, f
 iorwf LATA
 call extra0_l1
 xorwf v_udata_bank2_0, f
 goto code_l6
 incf WREG, F
extra0_l16:
 btfsc BSR, 4
 bsf LATA, 5
extra0_l18:
 banksel BSR
 bra extra0_l7
 call extra0_l7
 swapf PCLATH, W
PSECT extra1, class = CODE, delta = 2
 movlw high(((0Bh ^ K1) * (K0 * 06h)))
 movwi ++FSR0
extra1_l2:
 xorlw ((K1 << 2) >> 5)
 xorlw K0
 goto extra1_l2
 incf PCLATH, 1
 lsrf INTCON, 0
 bra extra1_l2
 end reset_vec
