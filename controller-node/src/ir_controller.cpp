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
    currentTemp = 70;  // Assume fireplace starts at 70°F
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
    if (currentTemp >= 80) {
        Serial.println("Manual HEAT UP: Already at max (80°F)");
        return;
    }
    Serial.printf("Manual HEAT UP: %d°F -> %d°F\n", currentTemp, currentTemp + 2);
    sendTempUpCode();
    currentTemp += 2;
}

void IRController::sendHeatDown() {
    if (currentTemp <= 60) {
        Serial.println("Manual HEAT DOWN: Already at min (60°F)");
        return;
    }
    Serial.printf("Manual HEAT DOWN: %d°F -> %d°F\n", currentTemp, currentTemp - 2);
    sendTempDownCode();
    currentTemp -= 2;
}

void IRController::sendTemp(int temp) {
    // Round to nearest even number and clamp to range
    temp = ((temp + 1) / 2) * 2;  // Round to even
    if (temp < 60) temp = 60;
    if (temp > 80) temp = 80;

    Serial.printf("Setting fireplace temperature to: %d°F (current: %d°F)\n", temp, currentTemp);

    // Send UP or DOWN codes to reach target temperature
    while (currentTemp != temp) {
        if (currentTemp < temp) {
            // Need to go up
            sendTempUpCode();
            currentTemp += 2;
            Serial.printf("  Sent TEMP UP -> now at %d°F\n", currentTemp);
        } else {
            // Need to go down
            sendTempDownCode();
            currentTemp -= 2;
            Serial.printf("  Sent TEMP DOWN -> now at %d°F\n", currentTemp);
        }
        delay(MIN_SEND_INTERVAL);  // Wait between commands
    }

    Serial.printf("Temperature set complete: %d°F\n", currentTemp);
}

void IRController::setCurrentTemp(int temp) {
    temp = ((temp + 1) / 2) * 2;  // Round to even
    if (temp < 60) temp = 60;
    if (temp > 80) temp = 80;
    currentTemp = temp;
}

void IRController::sendTempUpCode() {
    // Send the appropriate UP code based on current temperature
    switch(currentTemp) {
        case 60: sendRaw(IR_RAW_TEMP_UP_FROM_60, IR_RAW_TEMP_UP_FROM_60_LEN); break;
        case 62: sendRaw(IR_RAW_TEMP_UP_FROM_62, IR_RAW_TEMP_UP_FROM_62_LEN); break;
        case 64: sendRaw(IR_RAW_TEMP_UP_FROM_64, IR_RAW_TEMP_UP_FROM_64_LEN); break;
        case 66: sendRaw(IR_RAW_TEMP_UP_FROM_66, IR_RAW_TEMP_UP_FROM_66_LEN); break;
        case 68: sendRaw(IR_RAW_TEMP_UP_FROM_68, IR_RAW_TEMP_UP_FROM_68_LEN); break;
        case 70: sendRaw(IR_RAW_TEMP_UP_FROM_70, IR_RAW_TEMP_UP_FROM_70_LEN); break;
        case 72: sendRaw(IR_RAW_TEMP_UP_FROM_72, IR_RAW_TEMP_UP_FROM_72_LEN); break;
        case 74: sendRaw(IR_RAW_TEMP_UP_FROM_74, IR_RAW_TEMP_UP_FROM_74_LEN); break;
        case 76: sendRaw(IR_RAW_TEMP_UP_FROM_76, IR_RAW_TEMP_UP_FROM_76_LEN); break;
        case 78: sendRaw(IR_RAW_TEMP_UP_FROM_78, IR_RAW_TEMP_UP_FROM_78_LEN); break;
    }
}

void IRController::sendTempDownCode() {
    // Send the appropriate DOWN code based on current temperature
    switch(currentTemp) {
        case 80: sendRaw(IR_RAW_TEMP_DOWN_FROM_80, IR_RAW_TEMP_DOWN_FROM_80_LEN); break;
        case 78: sendRaw(IR_RAW_TEMP_DOWN_FROM_78, IR_RAW_TEMP_DOWN_FROM_78_LEN); break;
        case 76: sendRaw(IR_RAW_TEMP_DOWN_FROM_76, IR_RAW_TEMP_DOWN_FROM_76_LEN); break;
        case 74: sendRaw(IR_RAW_TEMP_DOWN_FROM_74, IR_RAW_TEMP_DOWN_FROM_74_LEN); break;
        case 72: sendRaw(IR_RAW_TEMP_DOWN_FROM_72, IR_RAW_TEMP_DOWN_FROM_72_LEN); break;
        case 70: sendRaw(IR_RAW_TEMP_DOWN_FROM_70, IR_RAW_TEMP_DOWN_FROM_70_LEN); break;
        case 68: sendRaw(IR_RAW_TEMP_DOWN_FROM_68, IR_RAW_TEMP_DOWN_FROM_68_LEN); break;
        case 66: sendRaw(IR_RAW_TEMP_DOWN_FROM_66, IR_RAW_TEMP_DOWN_FROM_66_LEN); break;
        case 64: sendRaw(IR_RAW_TEMP_DOWN_FROM_64, IR_RAW_TEMP_DOWN_FROM_64_LEN); break;
        case 62: sendRaw(IR_RAW_TEMP_DOWN_FROM_62, IR_RAW_TEMP_DOWN_FROM_62_LEN); break;
    }
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
