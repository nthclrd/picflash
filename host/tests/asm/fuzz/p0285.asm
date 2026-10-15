PROCESSOR 16F1789
#include <xc.inc>
K0 EQU 0x98
PSECT udata_bank0
v_udata_bank0_0: DS 4
v_udata_bank0_1: DS 4
v_udata_bank0_2: DS 3
PSECT udata_shr
v_udata_shr_0: DS 2
v_udata_shr_1: DS 4
v_udata_shr_2: DS 1
PSECT udata
v_udata_0: DS 1
v_udata_1: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 reset
 banksel v_udata_1
 iorlw (not 0x41)
 movlb 23
 xorwf BSR, 0
 andlw (low((~ K0)) shl 7)
code_l6:
 sublw K0
 bra code_l6
code_l8:
 movwi --FSR0
 end reset_vec
