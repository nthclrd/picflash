PROCESSOR 16F18877
#include <xc.inc>
K0 EQU ((33o shr 2) / 012h)
K1 EQU 011Dh
K2 EQU (low((202 shr 5)) - 0x14)
K3 EQU K1
PSECT udata_bank0
v_udata_bank0_0: DS 3
v_udata_bank0_1: DS 2
v_udata_bank0_2: DS 2
PSECT udata
v_udata_0: DS 1
PSECT udata_shr
v_udata_shr_0: DS 1
v_udata_shr_1: DS 1
PSECT udata_bank1
v_udata_bank1_0: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 subwfb FSR1H
code_l1:
 movlw K3
 addlw (0b1011 shl 4)
 sleep
 dw ((78 & (not K1)) mod 16o), ((low(061h) - (~ K3)) | (K1 mod 0Ch)), (high((223o >> 4)) + ((337o mod 05h) << 0))
 movwi FSR0++
 andlw 0123h
 addfsr FSR0, 4
 reset
 banksel LATA
 decf v_udata_bank0_2, F
PSECT extra0, class = CODE, delta = 2
 rrf v_udata_shr_1, f
 bcf v_udata_0, 6
 subwfb WREG, 0
extra0_l3:
 movf v_udata_bank0_1
 sublw (not ((267 shl 4) ^ (K3 - 0b1101100)))
 xorlw (low((not K2)) shr 4)
 movlb 2
 bra extra0_l3
extra0_l8:
 asrf 44o, 0
 movwf STATUS
 subwf v_udata_bank1_0, 0
 incfsz v_udata_bank0_1, w
 moviw --FSR1
 comf v_udata_bank0_1, w
 bcf v_udata_bank0_1, 5
 retfie
 movlp 27
PSECT extra1, class = CODE, delta = 2, reloc = 4
 nop
 clrf TRISA
 decf PCLATH, 1
 bsf v_udata_shr_1, 3
 movwi --FSR1
extra1_l5:
 bra extra1_l5
 asrf v_udata_bank0_0, 0
 lsrf v_udata_bank0_1, 0
 pagesel extra1_l5
 bra extra1_l5
 bra extra1_l5
 btfss TRISA, 7
 rlf WREG, w
 movf 0b11101, w
 iorlw (((K2 shr 1) and high(K2)) mod 5o)
 call extra0_l8
 btfss TRISA, 2
 movwf BSR
 clrf v_udata_0
 movlb 29
 dw (K2 & K1)
 rrf v_udata_0, F
 end reset_vec
