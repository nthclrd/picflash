PROCESSOR 16F1789
#include <xc.inc>
K0 EQU 24
K1 EQU 0xd3
S0 SET 74
S0 SET S0 + 1
PSECT udata_bank2
v_udata_bank2_0: DS 1
v_udata_bank2_1: DS 3
v_udata_bank2_2: DS 2
PSECT udata_shr
v_udata_shr_0: DS 3
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT code
start:
 rrf v_udata_bank2_2, w
 asrf TRISA
code_l2:
 btfsc v_udata_bank2_2, 0
 movwf FSR0L
 moviw -2[FSR1]
code_l5:
 andwf PCLATH, F
 call code_l2
 decf PORTA
 call start
 movlb 15
 decf STATUS, 0
code_l11:
 iorwf v_udata_bank2_1
code_l12:
 movwi --FSR0
 addfsr FSR1, 4
PSECT extra0, class = CODE, delta = 2
 call code_l2
extra0_l1:
 comf 0x6e, f
extra0_l2:
 bsf STATUS, 1
 sleep
 xorlw 0b1010010
extra0_l5:
 rlf LATA
 bsf TRISA, 2
 pagesel code_l5
 sublw ((293 - (~ 074h)) and K1)
 retfie
 bcf PORTA, 2
 btfss INTCON, 6
extra0_l12:
 retlw K1
 pagesel extra0_l2
 movlb 6
 moviw --FSR0
 moviw -31[FSR1]
 movlp 93
PSECT extra1, class = CODE, delta = 2
extra1_l0:
 dw (((0xfa - K0) << 0) shl 4)
 lsrf INTCON, 1
 xorlw (low((05Ch << 3)) + ((not 0xde) >> 4))
 btfsc PCLATH, 1
 call extra0_l12
 addfsr FSR0, 8
 callw
 pagesel extra0_l2
 xorlw (0b11100010 * (~ (K0 mod 0b1010)))
 bra extra1_l0
 movf v_udata_bank2_0, F
 nop
 subwfb TRISA, 0
 clrf v_udata_shr_0
 pagesel start
 bsf STATUS, 3
 addlw (((90 shr 3) shl 6) / 0x10)
extra1_l17:
 dw (S0 * 0b10011001), 55o
 bcf TRISA, 0
extra1_l19:
 movwf FSR1H
 movlp 67
 end reset_vec
