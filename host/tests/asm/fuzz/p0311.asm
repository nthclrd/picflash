PROCESSOR 16F1509
#include <xc.inc>
K0 EQU 0b10101101
K1 EQU (K0 and high((0b10100001 mod 0b1)))
K2 EQU 152
S0 SET 242o
S0 SET S0 + 1
PSECT udata_shr
v_udata_shr_0: DS 2
v_udata_shr_1: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 movlb 31
 iorlw (high(60) << 3)
 addfsr FSR0, 17
 addwf FSR0L, F
PSECT extra0, class = CODE, delta = 2, reloc = 4
extra0_l0:
 bra extra0_l0
extra0_l1:
 dw ((0x9c - K2) shl 7), 0A3h, S0
 moviw FSR1--
 sublw 171
 retlw (K0 - (0x26 mod 0b1100))
 movlp 60
PSECT extra1, class = CODE, delta = 2
 movwf INTCON
 goto extra0_l0
 incf FSR1H, f
extra1_l3:
 bcf WREG, 6
 swapf 01Dh, W
 bcf PORTA, 7
 clrf v_udata_shr_1
 movlp 83
 end reset_vec
