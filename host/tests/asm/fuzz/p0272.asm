PROCESSOR 16F18877
#include <xc.inc>
PSECT udata_bank2
v_udata_bank2_0: DS 3
v_udata_bank2_1: DS 4
v_udata_bank2_2: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 addlw (((232 / 13o) | 0b100100111) * ((61o - 098h) >> 1))
 bsf WREG, 1
 addfsr FSR1, -25
 lsrf v_udata_bank2_0, 0
 andlw (low(305o) and 136o)
 addlw (high((0b11011111 + 0b10111010)) + ((0x6e mod 0b111) shl 5))
 btfss v_udata_bank2_1, 5
 andwf v_udata_bank2_1, F
 bra start
 andlw ((0b11111000 | high(0AFh)) / 9)
 retlw (low((0b10111111 ^ 239)) shr 4)
 bcf FSR0L, 5
 end reset_vec
