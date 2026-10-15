PROCESSOR 16F18877
#include <xc.inc>
K0 EQU 435o
K1 EQU ((high(K0) and (K0 and 420o)) & (013h and (K0 mod 10o)))
K2 EQU (((K1 / 0b1110) + K1) / 0x11)
K3 EQU 0b100000101
PSECT udata_shr
v_udata_shr_0: DS 3
v_udata_shr_1: DS 3
v_udata_shr_2: DS 4
PSECT udata_bank2
v_udata_bank2_0: DS 4
PSECT reset_vec, class = CODE, delta = 2
reset_vec:
 goto start
PSECT isr_vec, class = CODE, delta = 2
 retfie
PSECT code
start:
 sublw (0b110101 | (156 shl 0))
code_l1:
 subwf WREG
 goto code_l1
 sublw (0Dh & (151 ^ 07Eh))
 btfsc STATUS, 1
 goto code_l1
 movwf TRISA
code_l7:
 iorlw (K3 / 06h)
 addfsr FSR1, 1
 btfss PCLATH, 1
PSECT extra0, class = CODE, delta = 2, reloc = 4
 pagesel start
 decfsz 0x13, 1
 addfsr FSR1, 29
 clrf v_udata_shr_0
 subwf v_udata_shr_1, w
 incfsz FSR0L
 call start
 movlb 0
 moviw FSR0--
 lslf WREG
 iorlw ((not (146o / 0b1000)) & high(0x115))
 clrf LATA
 end reset_vec
