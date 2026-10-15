PROCESSOR 16F18446
#include <xc.inc>
K0 EQU 157
PSECT udata_bank2
v_udata_bank2_0: DS 3
PSECT udata
v_udata_0: DS 1
v_udata_1: DS 3
PSECT udata_bank1
v_udata_bank1_0: DS 1
v_udata_bank1_1: DS 3
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 bcf v_udata_bank1_1, 5
 bcf TRISA, 4
code_l2:
 subwf LATA, 1
code_l3:
 andwf PCLATH, 1
code_l4:
 goto code_l2
 addlw ((310o ^ K0) ^ 098h)
 bra code_l4
 subwf v_udata_bank2_0, W
 moviw 10[FSR1]
 moviw ++FSR1
 rlf v_udata_bank1_1, 0
 incfsz WREG, F
 movwf LATA
 decf BSR, 0
 end reset_vec
