PROCESSOR 16F18446
#include <xc.inc>
K0 EQU high(((0x1c ^ 377o) & (not 0x10d)))
K1 EQU (K0 >> 0)
K2 EQU 0F3h
K3 EQU K0
S0 SET 03Bh
S0 SET S0 + 1
PSECT udata
v_udata_0: DS 1
v_udata_1: DS 4
v_udata_2: DS 4
PSECT udata_shr
v_udata_shr_0: DS 2
v_udata_shr_1: DS 4
v_udata_shr_2: DS 3
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT code
start:
 decf LATA, f
 rlf PCLATH, w
code_l2:
 sublw 0x30
 moviw FSR0--
 end reset_vec
