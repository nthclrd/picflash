PROCESSOR 16F18446
#include <xc.inc>
K0 EQU 01Ch
PSECT udata_bank0
v_udata_bank0_0: DS 4
v_udata_bank0_1: DS 3
v_udata_bank0_2: DS 3
PSECT udata_shr
v_udata_shr_0: DS 2
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT code
start:
 addwf TRISA
 call start
 clrf BSR
code_l3:
 retlw 0x4
 call start
 rrf v_udata_shr_0, F
 iorlw (K0 shl 3)
code_l7:
 movwf FSR1H
 goto code_l3
code_l9:
 asrf TRISA, W
code_l10:
 movlb 21
PSECT extra0, class = CODE, delta = 2
 goto code_l10
 comf PORTA, 1
 addfsr FSR1, -15
 lslf INTCON, f
 rlf FSR1H, W
 sublw 0b10110000
extra0_l6:
 decfsz v_udata_bank0_0, F
 comf STATUS
extra0_l8:
 movwi FSR1--
 movwf PORTA
 movwi FSR1++
 moviw --FSR1
 call code_l9
 addfsr FSR1, 6
 dw (364o + ((0xda + 75o) + 4)), (((K0 | K0) + 0b10110001) + K0), high((not (090h + K0)))
 movlb 53
 dw (not high(low(K0))), 0A5h, K0
extra0_l17:
 xorwf WREG, f
PSECT extra1, class = CODE, delta = 2
 movlb 45
 banksel FSR1H
 movlw (not ((5o / 0b1) | (20 / 17o)))
 reset
 swapf STATUS, F
extra1_l5:
 lsrf PCLATH, 0
 call code_l9
extra1_l7:
 banksel v_udata_shr_0
 movlb 42
extra1_l9:
 bra extra1_l7
 movlb 34
 sublw (~ K0)
 pagesel code_l7
 iorwf v_udata_shr_0, F
extra1_l14:
 addwfc TRISA, F
 movwf LATA
extra1_l16:
 bra extra1_l9
 movlp 19
 end reset_vec
