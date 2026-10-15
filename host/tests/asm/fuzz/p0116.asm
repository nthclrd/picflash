PROCESSOR 16F18877
#include <xc.inc>
K0 EQU 406o
K1 EQU (((K0 - K0) | (K0 / 0x9)) & K0)
K2 EQU ((K0 ^ (K0 >> 1)) and 0xba)
K3 EQU ((~ (K0 << 6)) shl 4)
S0 SET (((0A5h >> 4) >> 4) and 0C2h)
S0 SET S0 + 1
PSECT udata
v_udata_0: DS 4
v_udata_1: DS 3
PSECT udata_bank1
v_udata_bank1_0: DS 2
v_udata_bank1_1: DS 3
v_udata_bank1_2: DS 1
PSECT udata_bank0
v_udata_bank0_0: DS 4
v_udata_bank0_1: DS 3
PSECT udata_shr
v_udata_shr_0: DS 2
v_udata_shr_1: DS 2
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT code
start:
 bra start
 movwi FSR1++
 rrf 0b1111101, f
code_l3:
 incf PORTA, 1
 moviw ++FSR0
 movlw 145o
 btfss v_udata_0, 3
 decf v_udata_shr_0, F
 subwf v_udata_bank1_0, w
 bsf v_udata_shr_0, 2
 clrw
 banksel v_udata_bank0_0
 moviw FSR1--
 rrf BSR, 0
 nop
 subwf v_udata_bank0_1
 addlw S0
 bcf TRISA, 2
 end reset_vec
