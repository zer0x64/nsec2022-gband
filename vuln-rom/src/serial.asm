INCLUDE "constants.inc"

SERIAL_STATE_WAITING_TO_PRESS_A = 0
SERIAL_STATE_WAITING_FOR_CLIENT = 1
SERIAL_STATE_TRANSFERING = 2
SERIAL_STATE_TRANSFER_OVER = 3

SECTION FRAGMENT "Serial transfer", ROMX
RunSerialMode::
    ; Disable the PPU
    xor a
    ld [rLCDC], a

    ; We start without any scroll
    ld [shadowScrollX], a
    ld [shadowScrollY], a

    ; Clear Screen
    ld hl, _SCRN0
    ld bc, _SCRN1 - _SCRN0
    call MemSet

    ; Copy the tile map
    ld de, serialTileMap
    ld hl, _SCRN0
    ld bc, serialTileMap.end - serialTileMap
    call CopyToVRAM

    ld a, [isCgb]
    cp 1
    jr nz, .skipAttributeCopy

    ; GDMA the attribute map
    ; Change VRAM bank
    ld a, 1
    ld [rVBK], a

    ld de, serialAttributes
    ld hl, _SCRN0
    ld bc, serialAttributes.end - serialAttributes
    call CopyToVRAM

    ; Reset VRAM bank
    ld a, 0
    ld [rVBK], a

.skipAttributeCopy
    ; We disable every sprites
    xor a
    ld [shadowOAM], a

    ; We set the default game state
    ld a, SERIAL_STATE_WAITING_TO_PRESS_A
    ld [serialState], a

    ; Turn LDC on
    ld a, LCDC_DEFAULT
    ld [rLCDC], a
    ei

.loop
    ld a, [serialState]
    cp SERIAL_STATE_WAITING_TO_PRESS_A
    jr z, .waitingToPressA

    cp SERIAL_STATE_WAITING_FOR_CLIENT
    jr z, .waitingForClient

    cp SERIAL_STATE_TRANSFERING
    jr z, .waitingForClient

.waitingToPressA
    ; Check if connection
    ld a, [serialConnectionState]
    cp SERIAL_CONNECTION_STATE_EXTERNAL
    jr nz, :+

    ; Client is connected, update the state
    ld a, SERIAL_STATE_TRANSFERING
    ld [serialState], a
    jr .render
:
    ; Not connected yet
    ld a, SERIAL_CONNECTION_STATE_UNCONNECTED
    ld [serialConnectionState], a

    ; We update the joypad state
    call ReadJoypad

    ; We handle the buttons
    ld a, [joypadButtons]
    ld b, a
    ld a, [joypadButtonsOld]

    call GetNewlyPushedButtons

    ; We only check for the a button
    bit 0, a

    ; If a is pressed, start with internal clock
    jr z, :+
    ld a, SERIAL_STATE_WAITING_FOR_CLIENT
    ld [serialState], a
:
    ; Else, wait for connection with external clock
    ld a, SERIAL_CONNECTION_STATE_INTERNAL ; Tell the other to connect as internal
    ldh [rSB], a
    xor a
    ld [serialReceiveData], a
    ld a, SCF_START
    ldh [rSC], a
    
    jr .render

.render
    call WaitVblank

    jr .loop

.waitingForClient
    ld a, SERIAL_CONNECTION_STATE_INTERNAL
    ld [serialConnectionState], a

    ld a, SERIAL_CONNECTION_STATE_EXTERNAL ; Tell the other to connect as external
    ldh [rSB], a
    ld a, SCF_START | SCF_SOURCE
    ldh [rSC], a
    call WaitVblank

    ; Wait until the other player has connected
:
    ld a, [serialReceivedNewData]
    and a
    jr z, :-
    ld a, [serialReceiveData]
    and a
    jr nz, .render

.transfering
    xor a
    call SerialSendByte

    call ExchangeName
    jr .done

.done
    halt
    jr .done

SerialSendByte:
    ld [serialSendData], a
    ld a, [serialConnectionState]
    cp SERIAL_CONNECTION_STATE_INTERNAL
    ret nz
    ld a, SCF_START | SCF_SOURCE
    ldh [rSC], a
    ret

ExchangeName:
    push bc
    push de
    push hl
    call ExchangeNameLength

    ; Get max length, put it in b
    ld a, [playerNameLengthRam]
    ld b, a
    ld a, [otherPlayerNameLength]
    cp b
    jr c, .startExchanging
    ld b, a

.startExchanging
    ld hl, playerNameRam
    ld de, localVariables

.loop
    ; Exchange one byte
    ld a, [hli]
    ld [serialSendData], a
    call SerialSendByte
    call WaitVblank

:
    ld a, [serialReceivedNewData]
    and a
    jr z, :-
    ld a, [serialReceiveData]
    ld c, a

    ; Wait
    call WaitVblank

    ; Store the byte into the local variables
    ld a, c
    ld [de], a
    inc de

    ; Decrease the length counter
    dec b
    jr nz, .loop

    pop hl
    pop de
    pop bc
    ret

ExchangeNameLength:
    ld a, [playerNameLengthRam]
    call SerialSendByte
    call WaitVblank

:
    ld a, [serialReceivedNewData]
    and a
    jr z, :-
    ld a, [serialReceiveData]
    ld [otherPlayerNameLength], a

    ret

WaitVblank:
    ; Lock so we wait for the frame to end
    ld a, 1
    ld [waitForFrame], a;
.waitForFrame
    ; Wait until waitForFrame = 0, which is set by the VBlank handler
    ld a, [waitForFrame]
    cp 0
    jr nz, .waitForFrame
    ret

SECTION FRAGMENT "Serial transfer", ROMX, ALIGN[8]
serialTileMap:
INCBIN "res/serial_tilemap.bin"
.end

serialAttributes:
INCBIN "res/serial_attributes.bin"
.end

