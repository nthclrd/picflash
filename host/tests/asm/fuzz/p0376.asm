PROCESSOR 16F18877
#include <xc.inc>
K0 EQU (low(high(0b1010111)) shr 1)
K1 EQU K0
PSECT udata_bank2
v_udata_bank2_0: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 sublw 144o
 asrf 043h, 1
 lslf BSR, W
 incfsz PORTA, w
 btfsc FSR0L, 4
 call start
 btfss WREG, 2
 sublw (0x120 shr 7)
 movwf PCLATH
 bra start
code_l10:
 bra start
 incf FSR1H, f
 movwf STATUS
 andlw 0b100000011
 andlw high((~ (0xf5 | K1)))
 andwf FSR0L, 0
 movwi FSR1--
 btfsc TRISA, 1
 comf TRISA, w
code_l19:
 decf FSR1H, 0
 bcf FSR1H, 7
 end reset_vec
