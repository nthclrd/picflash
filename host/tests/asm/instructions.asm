; Every PIC16 enhanced mid-range instruction and operand form.
PROCESSOR 16F1789
#include <xc.inc>

PSECT udata_shr
tmp:    DS 1

PSECT reset_vec, class = CODE, delta = 2
reset_vec:
        goto    main

PSECT code
main:
        addwf   tmp, f
        addwf   tmp, w
        addwfc  0x21, f
        andwf   0x22, w
        asrf    0x23, f
        lslf    0x24, w
        lsrf    0x25, f
        clrf    0x26
        clrw
        comf    0x27, f
        decf    0x28, w
        decfsz  0x29, f
        incf    0x2A, w
        incfsz  0x2B, f
        iorwf   0x2C, w
        movf    0x2D, f
        movwf   0x2E
        rlf     0x2F, w
        rrf     0x30, f
        subwf   0x31, w
        subwfb  0x32, f
        swapf   0x33, w
        xorwf   0x34, f
        bcf     0x35, 0
        bsf     0x36, 7
        btfsc   0x37, 3
        btfss   0x38, 4
        addfsr  FSR0, 5
        addfsr  FSR1, -32
        addlw   0x10
        andlw   0x20
        bra     main
        brw
        call    sub
        callw
        goto    main
        iorlw   0x30
        movlb   5
        movlp   0x7F
        movlw   0xAA
        moviw   ++FSR0
        moviw   --FSR1
        moviw   FSR0++
        moviw   FSR1--
        moviw   7[FSR0]
        moviw   -1[FSR1]
        movwi   ++FSR1
        movwi   FSR0--
        movwi   31[FSR1]
        retfie
        retlw   0x55
        return
        sublw   0x01
        xorlw   0xFF
        clrwdt
        nop
        reset
        sleep
        tris    6
sub:    bra     sub
        end     reset_vec
