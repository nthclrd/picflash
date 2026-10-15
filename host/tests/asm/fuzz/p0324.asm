PROCESSOR 16F18446
#include <xc.inc>
PSECT udata
v_udata_0: DS 1
v_udata_1: DS 4
v_udata_2: DS 3
PSECT udata_bank1
v_udata_bank1_0: DS 3
v_udata_bank1_1: DS 3
PSECT udata_bank0
v_udata_bank0_0: DS 3
PSECT udata_shr
v_udata_shr_0: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 retfie
 bra start
 decfsz PCLATH, 0
 swapf v_udata_bank1_0, W
code_l4:
 addfsr FSR1, -2
 dw (266 / 06h), (012Bh shr 1), (((154o and 0Ah) - (not 0ECh)) * (~ 0x78))
code_l6:
 btfss BSR, 3
code_l7:
 incf WREG
 pagesel code_l7
 moviw 15[FSR0]
 movwf FSR1H
 movlb 37
 addfsr FSR1, -17
code_l13:
 btfss FSR1H, 2
 sublw 201o
 movf v_udata_bank1_0, 0
 btfss v_udata_bank0_0, 5
 end reset_vec
