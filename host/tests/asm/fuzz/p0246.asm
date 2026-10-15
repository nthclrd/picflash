PROCESSOR 16F1789
#include <xc.inc>
K0 EQU 0D0h
K1 EQU K0
K2 EQU K0
K3 EQU K0
PSECT udata_bank2
v_udata_bank2_0: DS 2
v_udata_bank2_1: DS 3
PSECT udata_bank0
v_udata_bank0_0: DS 2
PSECT udata_bank1
v_udata_bank1_0: DS 2
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 movlb 25
 reset
 sublw K3
 pagesel start
 dw K0, (322o << 0), (((~ 82) - (0b11010010 - 257o)) << 4)
 banksel BSR
 incfsz v_udata_bank2_0, 0
 addwfc PCLATH
 swapf v_udata_bank1_0, 1
 andlw ((0b100010110 | high(K2)) mod 0Dh)
 iorwf v_udata_bank1_0, f
code_l11:
 banksel v_udata_bank2_0
 movlb 17
 movlb 18
 dw ((K0 * high(115)) / 0b111)
 sublw low(0b10001001)
 addwf v_udata_bank2_1, F
code_l17:
 comf v_udata_bank0_0, W
 addwf FSR0L
 incfsz WREG, 0
code_l20:
 addwf PCLATH, w
 dw ((low(021h) / 0b1110) mod 3), (K1 mod 13o), high(436o)
 end reset_vec
