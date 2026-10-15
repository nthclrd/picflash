PROCESSOR 16F18446
#include <xc.inc>
S0 SET (((75o and 06Fh) << 6) + (214 + (0124h / 22o)))
S0 SET S0 + 1
PSECT udata_bank0
v_udata_bank0_0: DS 1
v_udata_bank0_1: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 bra start
 movf INTCON, 1
 movf BSR, F
 iorwf PCLATH, 1
 call start
 bsf v_udata_bank0_0, 5
 banksel v_udata_bank0_0
 addfsr FSR1, -24
 pagesel start
code_l9:
 return
 comf TRISA, 1
 movlw 232
 bsf FSR0L, 3
 dw (363o mod 6o)
 pagesel code_l9
 clrf LATA
 subwfb PORTA, W
 pagesel start
 xorlw (S0 & low(0xdc))
code_l19:
 bsf FSR1H, 1
 decfsz FSR0L, f
PSECT extra0, class = CODE, delta = 2
extra0_l0:
 bra extra0_l0
 iorlw 0x28
 bra extra0_l0
 movlb 62
 movlw (not low(S0))
extra0_l5:
 reset
 movwf WREG
 xorlw (132o mod 0xa)
 rlf STATUS, F
 call code_l9
 bsf FSR0L, 4
 banksel STATUS
 swapf PCLATH, w
 end reset_vec
