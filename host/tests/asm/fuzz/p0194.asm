PROCESSOR 16F18446
#include <xc.inc>
K0 EQU (~ high(71o))
K1 EQU low(04Dh)
PSECT udata_bank2
v_udata_bank2_0: DS 1
v_udata_bank2_1: DS 3
PSECT udata_bank1
v_udata_bank1_0: DS 4
PSECT udata_shr
v_udata_shr_0: DS 4
v_udata_shr_1: DS 4
v_udata_shr_2: DS 2
PSECT udata_bank0
v_udata_bank0_0: DS 2
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 moviw --FSR0
 comf PCLATH, f
 goto start
 brw
 goto start
 clrf INTCON
 rrf STATUS, W
 btfss v_udata_shr_2, 2
PSECT extra0, class = CODE, delta = 2
 btfsc v_udata_shr_0, 2
 addfsr FSR1, -21
 movlw high(((05Eh and 70o) shl 7))
 movwi --FSR0
 clrf v_udata_shr_0
 iorwf v_udata_shr_0
extra0_l6:
 movf LATA, F
 addfsr FSR0, 27
 movwf LATA
 banksel v_udata_shr_0
 rlf LATA, F
 xorlw (0b1010101 * 370o)
 movwf PORTA
 xorlw K0
 btfss FSR1H, 5
 addfsr FSR1, -24
 movlp 45
PSECT extra1, class = CODE, delta = 2
extra1_l0:
 bra extra1_l0
 rlf WREG
 decf v_udata_shr_1, F
 dw K1
 movwf FSR1H
 addfsr FSR0, 4
 addlw ((~ (0xed ^ 025h)) / 013h)
 clrf v_udata_shr_0
 addlw K1
extra1_l9:
 addfsr FSR1, -2
 goto extra1_l9
 movlp 47
 end reset_vec
