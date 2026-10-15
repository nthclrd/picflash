PROCESSOR 16F18446
#include <xc.inc>
K0 EQU 0xc0
K1 EQU (((K0 + K0) shr 5) shl 4)
K2 EQU (~ K0)
PSECT udata
v_udata_0: DS 4
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
code_l0:
 asrf TRISA, 0
 bra start
 banksel BSR
 movf STATUS
 pagesel code_l0
 btfss PORTA, 0
 decfsz STATUS, 0
 btfsc PCLATH, 7
 bra start
 banksel WREG
 sleep
 movlb 56
 addlw (K0 << 3)
 bra code_l0
PSECT extra0, class = CODE, delta = 2
 call code_l0
 subwf v_udata_0, F
 bsf FSR1H, 7
 movwf FSR1H
 call start
 bsf PORTA, 1
 goto start
extra0_l7:
 addlw (341o - (high(432o) shl 1))
 sublw 11o
 addwf BSR, f
 movwf v_udata_0
 banksel LATA
 andlw K0
 comf LATA, 0
 moviw FSR1++
 xorwf BSR, F
 swapf 077h
 pagesel extra0_l7
 clrf v_udata_0
 sleep
 movlp 75
PSECT extra1, class = CODE, delta = 2
extra1_l0:
 movlb 23
 xorlw 02h
 addfsr FSR1, -12
 banksel TRISA
 clrf BSR
extra1_l5:
 bra extra1_l5
 addlw ((254o << 7) and (not 301o))
 goto extra1_l0
 movwf v_udata_0
extra1_l9:
 addlw low(K1)
extra1_l10:
 sublw (~ ((0xef >> 6) | K0))
 addlw low(low((K0 - K2)))
 clrw
 asrf FSR0L, f
 goto extra1_l10
 banksel PCLATH
 movlp 43
 end reset_vec
