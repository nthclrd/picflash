PROCESSOR 16F1789

; CONFIG1
  CONFIG  FOSC = INTOSC         ; Oscillator Selection (INTOSC oscillator: I/O function on CLKIN pin)
  CONFIG  WDTE = OFF            ; Watchdog Timer Enable (WDT disabled)
  CONFIG  PWRTE = OFF           ; Power-up Timer Enable (PWRT disabled)
  CONFIG  MCLRE = ON            ; MCLR Pin Function Select (MCLR/VPP pin function is MCLR)
  CONFIG  CP = OFF              ; Flash Program Memory Code Protection (Program memory code protection is disabled)
  CONFIG  CPD = OFF             ; Data Memory Code Protection (Data memory code protection is disabled)
  CONFIG  BOREN = ON            ; Brown-out Reset Enable (Brown-out Reset enabled)
  CONFIG  CLKOUTEN = OFF        ; Clock Out Enable (CLKOUT function is disabled. I/O or oscillator function on the CLKOUT pin)
  CONFIG  IESO = ON             ; Internal/External Switchover (Internal/External Switchover mode is enabled)
  CONFIG  FCMEN = ON            ; Fail-Safe Clock Monitor Enable (Fail-Safe Clock Monitor is enabled)

; CONFIG2
  CONFIG  WRT = OFF             ; Flash Memory Self-Write Protection (Write protection off)
  CONFIG  VCAPEN = OFF          ; Voltage Regulator Capacitor Enable bit (Vcap functionality is disabled on RA6.)
  CONFIG  PLLEN = ON            ; PLL Enable (4x PLL enabled)
  CONFIG  STVREN = ON           ; Stack Overflow/Underflow Reset Enable (Stack Overflow or Underflow will cause a Reset)
  CONFIG  BORV = LO             ; Brown-out Reset Voltage Selection (Brown-out Reset Voltage (Vbor), low trip point selected.)
  CONFIG  LPBOR = OFF           ; Low Power Brown-Out Reset Enable Bit (Low power brown-out is disabled)
  CONFIG  DEBUG = OFF           ; In-Circuit Debugger Mode (In-Circuit Debugger disabled, ICSPCLK and ICSPDAT are general purpose I/O pins)
  CONFIG  LVP = ON            ; Low-Voltage Programming Enable (High-voltage on MCLR/VPP must be used for programming)

#include <xc.inc>

PSECT udata_shr
counter_l:
        DS      1
counter_h:
	DS      1
counter_hh:
	DS      1

PSECT reset_vec, class = CODE, delta = 2
reset_vec:
        goto    start

PSECT isr_vec, class = CODE, delta = 2
        retfie

PSECT code

delay:  movlw   0xf8
        movwf   counter_hh
        clrf    counter_h
        clrf    counter_l
delay_loop:
        incfsz  counter_l, f
        goto    delay_loop
        incfsz  counter_h, f
        goto    delay_loop
        incfsz  counter_hh, f
        goto    delay_loop
        return

start:  banksel OSCCON
        movlw   0xf8
        movwf   OSCCON
        banksel ANSELA
        clrf    ANSELA
        banksel TRISA
        clrf    TRISA
        clrf    BSR
loop:   bsf     PORTA, 0
        call    delay
        bcf     PORTA, 0
        call    delay
        goto    loop

        end	reset_vec
