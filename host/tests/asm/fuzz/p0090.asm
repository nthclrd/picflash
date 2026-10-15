PROCESSOR 16F18446
#include <xc.inc>
PSECT udata_bank0
v_udata_bank0_0: DS 4
v_udata_bank0_1: DS 4
PSECT udata_bank2
v_udata_bank2_0: DS 4
v_udata_bank2_1: DS 3
v_udata_bank2_2: DS 2
PSECT udata
v_udata_0: DS 3
v_udata_1: DS 2
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 movlw ((164 >> 1) / 0b1110)
code_l1:
 addfsr FSR0, 28
code_l2:
 decfsz v_udata_bank2_1, 1
 bra start
PSECT extra0, class = CODE, delta = 2
 iorwf BSR, W
 addwfc v_udata_1, w
extra0_l2:
 movlb 56
extra0_l3:
 sublw 0x108
extra0_l4:
 subwf INTCON, F
 pagesel extra0_l2
 swapf v_udata_1, w
 movlp 114
PSECT extra1, class = CODE, delta = 2, reloc = 4
 addfsr FSR0, -19
extra1_l1:
 movwf FSR0L
 decfsz v_udata_bank2_1, 0
 decf v_udata_bank0_1, 1
 andwf PCLATH, W
extra1_l5:
 btfsc TRISA, 7
 xorwf v_udata_bank2_1, F
 sublw 0b1101000
 bra extra1_l5
 bcf v_udata_bank2_0, 5
 clrw
 end reset_vec
