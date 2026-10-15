PROCESSOR 16F1509
#include <xc.inc>
K0 EQU 0xc7
PSECT udata_bank2
v_udata_bank2_0: DS 4
v_udata_bank2_1: DS 1
PSECT udata_bank0
v_udata_bank0_0: DS 1
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 clrwdt
 xorlw ((~ (273 shr 2)) / 2)
 movlb 20
 andlw (~ (~ (K0 / 4)))
code_l4:
 pagesel code_l4
 addwfc WREG, W
 banksel v_udata_bank2_0
 btfsc FSR0L, 3
 andlw (~ 200o)
 pagesel code_l4
 movlw (~ K0)
 decf PORTA, W
 movwi FSR1++
PSECT extra0, class = CODE, delta = 2
 movlb 8
 movlw (((370o shr 0) * low(0x7)) & 250o)
extra0_l2:
 swapf PORTA, f
 bsf v_udata_bank2_1, 7
 incfsz 0b11100, 0
 iorlw K0
 addfsr FSR1, -17
 incfsz BSR, 1
 clrf v_udata_bank2_1
 reset
extra0_l10:
 movlb 29
 banksel WREG
 goto code_l4
 clrwdt
 clrf PCLATH
 movlb 19
 call start
 pagesel extra0_l10
 lsrf PORTA, 1
 iorlw K0
 retlw 48
 addfsr FSR1, 18
 movlp 21
PSECT extra1, class = CODE, delta = 2, reloc = 4
 andwf FSR1H, w
extra1_l1:
 bsf v_udata_bank2_0, 5
 decfsz FSR0L
 movlp 116
 end reset_vec
