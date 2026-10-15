PROCESSOR 16F1509
#include <xc.inc>
K0 EQU (10o shr 3)
K1 EQU (((K0 shl 4) shl 1) / 0xe)
K2 EQU (not 0b11001111)
S0 SET K0
S0 SET S0 + 1
PSECT udata_bank0
v_udata_bank0_0: DS 3
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT code
start:
code_l0:
 movlb 31
 retlw K0
code_l2:
 movlb 15
 clrw
 goto start
 iorlw (low((445o - 04Fh)) and low(80))
 lslf TRISA
PSECT extra0, class = CODE, delta = 2, reloc = 4
extra0_l0:
 xorlw 245o
 pagesel code_l0
 bra extra0_l0
 movf PCLATH, w
 btfss FSR0L, 2
extra0_l5:
 bcf PCLATH, 4
 movwf TRISA
 addwf FSR0L, f
 clrf FSR1H
extra0_l9:
 iorwf FSR1H, 0
extra0_l10:
 addwf v_udata_bank0_0, W
 bcf FSR1H, 5
extra0_l12:
 movlb 20
 subwf BSR
 rrf TRISA, f
 movf TRISA, 0
 sublw K1
 pagesel extra0_l0
 lsrf FSR0L, F
 banksel TRISA
 retfie
 subwfb PORTA, w
 movlp 61
PSECT extra1, class = CODE, delta = 2, reloc = 4
 comf BSR, F
extra1_l1:
 addfsr FSR1, -22
 decf BSR, f
 banksel FSR1H
 addwf INTCON, W
 xorlw (((K0 & K1) >> 3) mod 0xf)
 movlb 2
 subwf STATUS, w
 addwf 32o
extra1_l9:
 movlw (high((0b100100001 / 06h)) ^ (0DBh mod 0b1011))
 decfsz STATUS
 end reset_vec
