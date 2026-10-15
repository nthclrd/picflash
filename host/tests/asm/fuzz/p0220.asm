PROCESSOR 16F18877
#include <xc.inc>
K0 EQU 13o
K1 EQU (52 | high(K0))
K2 EQU ((0x40 ^ 35o) << 2)
S0 SET ((not (K2 and 0x53)) * ((52o >> 2) / 0b1101))
S0 SET S0 + 1
PSECT udata
v_udata_0: DS 4
PSECT udata_bank2
v_udata_bank2_0: DS 4
PSECT udata_shr
v_udata_shr_0: DS 3
v_udata_shr_1: DS 2
v_udata_shr_2: DS 1
PSECT udata_bank0
v_udata_bank0_0: DS 1
v_udata_bank0_1: DS 4
v_udata_bank0_2: DS 4
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
code_l0:
 movlb 21
code_l1:
 xorlw K2
code_l2:
 xorlw (not 0123h)
 goto code_l2
PSECT extra0, class = CODE, delta = 2
extra0_l0:
 decf STATUS
 banksel PORTA
 addwf PORTA, f
extra0_l3:
 bra extra0_l0
 addfsr FSR1, 25
 movlp 70
 end reset_vec
