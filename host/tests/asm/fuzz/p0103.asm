PROCESSOR 16F1789
#include <xc.inc>
S0 SET 0BBh
S0 SET S0 + 1
PSECT udata_shr
v_udata_shr_0: DS 2
PSECT udata_bank0
v_udata_bank0_0: DS 2
v_udata_bank0_1: DS 3
v_udata_bank0_2: DS 2
PSECT udata_bank2
v_udata_bank2_0: DS 3
v_udata_bank2_1: DS 2
v_udata_bank2_2: DS 2
PSECT udata
v_udata_0: DS 4
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 pagesel start
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 lslf WREG
 btfss LATA, 4
 banksel BSR
 incf FSR1H
 goto start
 moviw FSR1++
 banksel TRISA
 clrf BSR
code_l8:
 addfsr FSR1, 30
 decf INTCON, 1
 retlw 0x6b
code_l11:
 retlw S0
code_l12:
 movlb 30
 movf v_udata_bank2_0, 0
 movlb 19
 decfsz BSR, 1
 addwf LATA, w
 banksel TRISA
 bra code_l12
 movf 0x40, W
PSECT extra0, class = CODE, delta = 2
 rlf v_udata_bank2_1
extra0_l1:
 andlw ((S0 mod 0b1110) * ((63o | S0) / 0Ch))
 goto code_l8
 movwi --FSR0
 retlw S0
 bcf v_udata_0, 1
 addfsr FSR0, 30
 clrf WREG
 addwf PCLATH, w
 lsrf INTCON
 btfsc PORTA, 7
extra0_l11:
 retlw ((30 ^ (172o and 105o)) + 103)
 retlw ((not (~ S0)) + ((S0 << 4) + (0x51 ^ 018h)))
extra0_l13:
 bcf v_udata_bank2_1, 1
 movlb 26
extra0_l15:
 banksel STATUS
 movwf v_udata_shr_0
 movlb 0
 bcf STATUS, 1
 dw 0xe0, S0
 pagesel code_l11
 movwi ++FSR0
 xorlw (((291 >> 0) & 40o) / 06h)
 xorwf v_udata_bank2_1, 1
PSECT extra1, class = CODE, delta = 2, reloc = 4
 subwfb v_udata_bank2_0, F
 call code_l8
 dw 24o, low(((~ 0b1100011) ^ (0b11100110 << 0))), ((low(335o) / 0x2) / 0xe)
 end reset_vec
