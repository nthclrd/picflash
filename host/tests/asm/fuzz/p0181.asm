PROCESSOR 16F18446
#include <xc.inc>
K0 EQU 0b10000111
K1 EQU (((K0 >> 7) - (K0 mod 9)) & ((K0 / 3o) and K0))
K2 EQU ((K0 >> 5) & 140o)
PSECT udata_bank0
v_udata_bank0_0: DS 3
PSECT udata_bank1
v_udata_bank1_0: DS 4
v_udata_bank1_1: DS 2
PSECT udata_shr
v_udata_shr_0: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT code
start:
 goto start
 banksel STATUS
 banksel PORTA
code_l3:
 pagesel start
 andwf PORTA, f
PSECT extra0, class = CODE, delta = 2, reloc = 4
extra0_l0:
 clrf TRISA
 banksel PCLATH
 bra extra0_l0
 btfss BSR, 5
 end reset_vec
