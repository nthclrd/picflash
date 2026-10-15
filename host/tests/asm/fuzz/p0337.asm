PROCESSOR 16F18877
#include <xc.inc>
K0 EQU 0b11111010
K1 EQU (not (~ (K0 + 54)))
S0 SET ((high(0b100111) / 0b1) << 3)
S0 SET S0 + 1
PSECT udata
v_udata_0: DS 2
PSECT udata_bank2
v_udata_bank2_0: DS 4
v_udata_bank2_1: DS 4
PSECT udata_bank0
v_udata_bank0_0: DS 4
PSECT udata_shr
v_udata_shr_0: DS 1
v_udata_shr_1: DS 3
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 banksel v_udata_shr_0
 addfsr FSR1, 22
 rlf v_udata_bank2_0, 1
code_l3:
 decfsz v_udata_shr_0, w
 andwf 0x16, 0
code_l5:
 bcf FSR0L, 3
 goto code_l3
 iorwf STATUS, 1
 bra start
 decf v_udata_bank2_1, 0
 swapf FSR0L, W
 addlw (06Ah mod 0b1100)
code_l12:
 movlw S0
 bra code_l3
 goto code_l12
 movwi ++FSR1
 movf v_udata_shr_1, F
 sublw 0x89
 addfsr FSR0, -11
 goto start
PSECT extra0, class = CODE, delta = 2, reloc = 4
 goto code_l12
 banksel v_udata_shr_0
 movwi 0[FSR0]
 incf 03Bh, F
extra0_l4:
 xorlw (K0 << 0)
extra0_l5:
 btfss v_udata_bank2_1, 1
extra0_l6:
 addwfc FSR1H, W
 movwf INTCON
 pagesel code_l5
 retlw (~ 0A8h)
 addfsr FSR1, -11
 addlw (high((246o shl 6)) >> 1)
 addfsr FSR1, -2
 comf v_udata_bank2_1, w
 movwf PORTA
 movwf FSR0L
extra0_l16:
 btfsc v_udata_0, 2
PSECT extra1, class = CODE, delta = 2, reloc = 4
 addfsr FSR1, 17
 addfsr FSR1, 22
 addfsr FSR1, -15
 goto code_l3
 addlw S0
 goto extra0_l16
 movlp 75
 end reset_vec
