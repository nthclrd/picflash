PROCESSOR 16F1509
#include <xc.inc>
K0 EQU ((168 ^ (065h | 0123h)) << 3)
K1 EQU (324o mod 01h)
K2 EQU (~ K0)
K3 EQU (((K0 & 0b11) >> 1) / 15)
S0 SET ((K0 shr 6) << 0)
S0 SET S0 + 1
PSECT udata_bank1
v_udata_bank1_0: DS 1
PSECT udata_bank0
v_udata_bank0_0: DS 3
v_udata_bank0_1: DS 3
v_udata_bank0_2: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT code
start:
 call start
 bra start
 addfsr FSR0, 18
 retfie
 incf WREG, 0
 retlw high((0b10101101 * (198 / 7)))
 call start
 retlw (0b100000010 shr 1)
 addwf BSR
 movwi --FSR1
 addfsr FSR1, 3
 incf v_udata_bank1_0, f
 end reset_vec
