PROCESSOR 16F18877
#include <xc.inc>
K0 EQU low(((096h * 0x96) & 437o))
K1 EQU ((K0 + low(0C4h)) ^ 294)
S0 SET ((K1 / 1o) and (08h >> 7))
S0 SET S0 + 1
PSECT udata
v_udata_0: DS 4
v_udata_1: DS 1
PSECT udata_bank1
v_udata_bank1_0: DS 2
v_udata_bank1_1: DS 2
v_udata_bank1_2: DS 2
PSECT udata_bank2
v_udata_bank2_0: DS 1
v_udata_bank2_1: DS 1
v_udata_bank2_2: DS 4
PSECT udata_bank0
v_udata_bank0_0: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
code_l0:
 bra start
 andlw (0x6d mod 12)
 iorwf v_udata_bank2_0, w
 bra start
 pagesel start
 addfsr FSR1, -14
 call start
 sublw high(0xef)
 call code_l0
 banksel PCLATH
 addwf v_udata_bank1_1, 1
 movwi --FSR1
 nop
code_l13:
 pagesel code_l0
PSECT extra0, class = CODE, delta = 2, reloc = 4
 banksel v_udata_bank1_2
extra0_l1:
 movlw high(low(K0))
 bra extra0_l1
 lslf BSR
 banksel FSR1H
extra0_l5:
 asrf v_udata_bank1_0
 andlw 0BFh
 clrf v_udata_bank2_0
 movwf v_udata_1
 incf WREG, w
 goto extra0_l1
 btfsc TRISA, 7
 movlp 106
 end reset_vec
