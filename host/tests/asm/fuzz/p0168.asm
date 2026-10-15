PROCESSOR 16F18446
#include <xc.inc>
K0 EQU 443o
K1 EQU 0x128
K2 EQU (K0 | K1)
PSECT udata_bank0
v_udata_bank0_0: DS 3
PSECT udata_bank2
v_udata_bank2_0: DS 2
v_udata_bank2_1: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT code
start:
 bra start
 banksel TRISA
code_l2:
 call code_l2
 goto start
 movwf TRISA
 pagesel code_l2
code_l6:
 dw (K1 * K0), (((K2 + 0x97) / 0b1) / 05h), K1
 subwfb TRISA, F
 moviw ++FSR1
 xorwf STATUS, w
code_l10:
 andlw K1
 decfsz STATUS, w
 movlb 36
code_l13:
 rlf FSR0L, 1
 andlw low(0xaa)
 movlw 233
 iorwf PORTA, w
code_l17:
 rrf v_udata_bank0_0, F
 addfsr FSR1, 0
 pagesel code_l13
code_l20:
 addwfc FSR0L, F
 xorlw K0
PSECT extra0, class = CODE, delta = 2, reloc = 4
 decf PORTA, w
 incf LATA, w
 andwf v_udata_bank2_0, w
 movlb 9
 movlp 20
 end reset_vec
