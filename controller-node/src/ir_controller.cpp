/*
 * IR Controller Implementation
 * Handles IR transmission using raw timing data for non-standard protocols
 * Includes state tracking for light and timer controls
 */

#include "ir_controller.h"
#include "config.h"

IRController::IRController(uint16_t sendPin) : irsend(sendPin) {
    lastSendTime = 0;
    lightLevel = 0;  // Assume light starts off
    timerState = 0;  // Assume timer starts off
}

void IRController::begin() {
    irsend.begin();
    Serial.println("IR Controller initialized (raw mode)");
    Serial.println("  Light level: OFF, Timer: OFF");
}

bool IRController::canSend() {
    unsigned long now = millis();
    if (now - lastSendTime >= MIN_SEND_INTERVAL) {
        lastSendTime = now;
        return true;
    }
    return false;
}

void IRController::sendRaw(const uint16_t* data, uint16_t len) {
    if (!canSend()) {
        Serial.println("IR send rate limited");
        return;
    }

    // Send signal 3 times for reliability (KY-005 has limited power)
    for (int i = 0; i < 3; i++) {
        irsend.sendRaw(data, len, IR_SEND_FREQ);
        delay(50);  // Small gap between transmissions
    }
    Serial.printf("IR raw data sent (%d values, 3x repeat)\n", len);
}

// ============================================================================
// Power Controls
// ============================================================================

void IRController::sendOn() {
    Serial.println(">>> Sending FIREPLACE ON <<<");
    sendRaw(IR_RAW_POWER_ON, IR_RAW_POWER_ON_LEN);
}

void IRController::sendOff() {
    Serial.println(">>> Sending FIREPLACE OFF <<<");
    sendRaw(IR_RAW_POWER_OFF, IR_RAW_POWER_OFF_LEN);
}

// ============================================================================
// Heat Controls
// ============================================================================

void IRController::sendHeatOn() {
    Serial.println("Sending HEAT ON");
    sendRaw(IR_RAW_HEAT_ON, IR_RAW_HEAT_ON_LEN);
}

void IRController::sendHeatOff() {
    Serial.println("Sending HEAT OFF");
    sendRaw(IR_RAW_HEAT_OFF, IR_RAW_HEAT_OFF_LEN);
}

void IRController::sendHeatUp() {
    Serial.println("Sending HEAT UP");
    sendRaw(IR_RAW_HEAT_UP, IR_RAW_HEAT_UP_LEN);
}

void IRController::sendHeatDown() {
    Serial.println("Sending HEAT DOWN");
    sendRaw(IR_RAW_HEAT_DOWN, IR_RAW_HEAT_DOWN_LEN);
}

// ============================================================================
// Light Controls (state-dependent)
// Light cycles: off(0) -> 4 -> 3 -> 2 -> 1 -> off(0)
// ============================================================================

void IRController::setLightLevel(uint8_t level) {
    if (level <= 4) {
        lightLevel = level;
        Serial.printf("Light level set to: %d\n", lightLevel);
    }
}

void IRController::sendLightCode() {
    // Send the appropriate code based on current state
    switch (lightLevel) {
        case 0:  // Currently off, will go to 4
            Serial.println("Sending LIGHT (from OFF -> 4)");
            sendRaw(IR_RAW_LIGHT_FROM_OFF, IR_RAW_LIGHT_FROM_OFF_LEN);
            break;
        case 4:  // Currently 4, will go to 3
            Serial.println("Sending LIGHT (from 4 -> 3)");
            sendRaw(IR_RAW_LIGHT_FROM_4, IR_RAW_LIGHT_FROM_4_LEN);
            break;
        case 3:  // Currently 3, will go to 2
            Serial.println("Sending LIGHT (from 3 -> 2)");
            sendRaw(IR_RAW_LIGHT_FROM_3, IR_RAW_LIGHT_FROM_3_LEN);
            break;
        case 2:  // Currently 2, will go to 1
            Serial.println("Sending LIGHT (from 2 -> 1)");
            sendRaw(IR_RAW_LIGHT_FROM_2, IR_RAW_LIGHT_FROM_2_LEN);
            break;
        case 1:  // Currently 1, will go to off
            Serial.println("Sending LIGHT (from 1 -> OFF)");
            sendRaw(IR_RAW_LIGHT_FROM_1, IR_RAW_LIGHT_FROM_1_LEN);
            break;
    }
}

void IRController::sendLightToggle() {
    // Send the code for current state
    sendLightCode();

    // Update internal state to next value in cycle
    // Cycle: 0 -> 4 -> 3 -> 2 -> 1 -> 0
    switch (lightLevel) {
        case 0: lightLevel = 4; break;
        case 4: lightLevel = 3; break;
        case 3: lightLevel = 2; break;
        case 2: lightLevel = 1; break;
        case 1: lightLevel = 0; break;
    }
    Serial.printf("Light level now: %d\n", lightLevel);
}

// ============================================================================
// Timer Controls (state-dependent)
// Timer cycles: off(0) -> 0.5hr(1) -> 1hr(2) -> 2hr(3) -> ... -> 9hr(10) -> off(0)
// ============================================================================

void IRController::setTimerState(uint8_t state) {
    if (state <= 10) {
        timerState = state;
        Serial.printf("Timer state set to: %s\n", getTimerString());
    }
}

const char* IRController::getTimerString() const {
    static const char* timerStrings[] = {
        "OFF", "0.5hr", "1hr", "2hr", "3hr", "4hr", "5hr", "6hr", "7hr", "8hr", "9hr"
    };
    if (timerState <= 10) {
        return timerStrings[timerState];
    }
    return "?";
}

void IRController::sendTimerCode() {
    // Send the appropriate code based on current state
    switch (timerState) {
        case 0:  // Off -> 0.5hr
            Serial.println("Sending TIMER (from OFF -> 0.5hr)");
            sendRaw(IR_RAW_TIMER_FROM_OFF, IR_RAW_TIMER_FROM_OFF_LEN);
            break;
        case 1:  // 0.5hr -> 1hr
            Serial.println("Sending TIMER (from 0.5hr -> 1hr)");
            sendRaw(IR_RAW_TIMER_FROM_0_5, IR_RAW_TIMER_FROM_0_5_LEN);
            break;
        case 2:  // 1hr -> 2hr
            Serial.println("Sending TIMER (from 1hr -> 2hr)");
            sendRaw(IR_RAW_TIMER_FROM_1, IR_RAW_TIMER_FROM_1_LEN);
            break;
        case 3:  // 2hr -> 3hr
            Serial.println("Sending TIMER (from 2hr -> 3hr)");
            sendRaw(IR_RAW_TIMER_FROM_2, IR_RAW_TIMER_FROM_2_LEN);
            break;
        case 4:  // 3hr -> 4hr
            Serial.println("Sending TIMER (from 3hr -> 4hr)");
            sendRaw(IR_RAW_TIMER_FROM_3, IR_RAW_TIMER_FROM_3_LEN);
            break;
        case 5:  // 4hr -> 5hr
            Serial.println("Sending TIMER (from 4hr -> 5hr)");
            sendRaw(IR_RAW_TIMER_FROM_4, IR_RAW_TIMER_FROM_4_LEN);
            break;
        case 6:  // 5hr -> 6hr
            Serial.println("Sending TIMER (from 5hr -> 6hr)");
            sendRaw(IR_RAW_TIMER_FROM_5, IR_RAW_TIMER_FROM_5_LEN);
            break;
        case 7:  // 6hr -> 7hr
            Serial.println("Sending TIMER (from 6hr -> 7hr)");
            sendRaw(IR_RAW_TIMER_FROM_6, IR_RAW_TIMER_FROM_6_LEN);
            break;
        case 8:  // 7hr -> 8hr
            Serial.println("Sending TIMER (from 7hr -> 8hr)");
            sendRaw(IR_RAW_TIMER_FROM_7, IR_RAW_TIMER_FROM_7_LEN);
            break;
        case 9:  // 8hr -> 9hr
            Serial.println("Sending TIMER (from 8hr -> 9hr)");
            sendRaw(IR_RAW_TIMER_FROM_8, IR_RAW_TIMER_FROM_8_LEN);
            break;
        case 10:  // 9hr -> OFF
            Serial.println("Sending TIMER (from 9hr -> OFF)");
            sendRaw(IR_RAW_TIMER_FROM_9, IR_RAW_TIMER_FROM_9_LEN);
            break;
    }
}

void IRController::sendTimerToggle() {
    // Send the code for current state
    sendTimerCode();

    // Update internal state to next value in cycle
    // Cycle: 0 -> 1 -> 2 -> ... -> 10 -> 0
    timerState = (timerState + 1) % 11;
    Serial.printf("Timer now: %s\n", getTimerString());
}
