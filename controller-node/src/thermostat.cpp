/*
 * Thermostat Logic Implementation
 * Implements temperature control with hysteresis
 */

#include "thermostat.h"
#include "config.h"
#include <Preferences.h>

static Preferences preferences;

Thermostat::Thermostat(IRController* irController) : ir(irController) {
    targetTemp = DEFAULT_TARGET_TEMP;
    hysteresis = DEFAULT_HYSTERESIS;
    mode = ThermostatMode::OFF;
    state = ThermostatState::IDLE;
    currentTemp = 0;
    currentHumidity = 0;
    fireplaceOn = false;
    fireplaceOffset = 4;  // Default +4°F, will be loaded from EEPROM in begin()
    lastSensorUpdate = 0;
    lastStateChange = 0;
    minCycleTime = MIN_CYCLE_TIME;
    sensorStaleTimeout = SENSOR_STALE_TIMEOUT;

    // Hold mode
    holdActive = false;
    holdStartTime = 0;
    holdDuration = HOLD_DURATION_MS;

    // Runtime safety
    heatingStartTime = 0;
    cooldownStartTime = 0;
    inCooldown = false;

    // Settings persistence
    lastSettingsChange = 0;
    settingsPendingSave = false;

    // Temperature trend detection
    previousTemp = 0;
    lastTrendSample = 0;
    trendDirection = 0;
    consecutiveTrend = 0;
}

void Thermostat::begin() {
    loadSettings();
    Serial.println("Thermostat initialized");
    Serial.printf("  Target: %.1f°F, Hysteresis: %.1f°F\n", targetTemp, hysteresis);
    Serial.printf("  Mode: %s\n", getModeString());
    Serial.printf("  Fireplace offset: +%d°F\n", fireplaceOffset);
    Serial.printf("  Min cycle time: %lu ms\n", minCycleTime);
    Serial.printf("  Max runtime: %lu ms (%lu hours)\n", (unsigned long)MAX_RUNTIME_MS, (unsigned long)MAX_RUNTIME_MS / 3600000);
    Serial.printf("  Hold duration: %lu ms (%lu minutes)\n", (unsigned long)HOLD_DURATION_MS, (unsigned long)HOLD_DURATION_MS / 60000);
}

void Thermostat::setTargetTemp(float temp) {
    // Clamp to reasonable range (matching fireplace remote: 60-84)
    if (temp < 60.0) temp = 60.0;
    if (temp > 84.0) temp = 84.0;

    if (targetTemp != temp) {
        targetTemp = temp;
        Serial.printf("Target temperature set to: %.1f°F\n", targetTemp);
        markSettingsChanged();
    }
}

void Thermostat::setHysteresis(float hyst) {
    // Clamp to reasonable range
    if (hyst < 0.5) hyst = 0.5;
    if (hyst > 5.0) hyst = 5.0;

    if (hysteresis != hyst) {
        hysteresis = hyst;
        Serial.printf("Hysteresis set to: %.1f°F\n", hysteresis);
        markSettingsChanged();
    }
}

void Thermostat::setMode(ThermostatMode newMode) {
    if (mode != newMode) {
        mode = newMode;
        Serial.printf("Thermostat mode set to: %s\n", getModeString());
        markSettingsChanged();

        // If turning off, turn off fireplace
        if (mode == ThermostatMode::OFF && fireplaceOn) {
            turnFireplaceOff();
        }

        // Reset state
        if (mode == ThermostatMode::OFF) {
            state = ThermostatState::IDLE;
            // Exit hold mode when thermostat is turned off
            holdActive = false;
        }
    }
}

void Thermostat::updateSensorData(float temperature, float humidity) {
    currentTemp = temperature;
    currentHumidity = humidity;
    lastSensorUpdate = millis();

    Serial.printf("Sensor update: %.1f°F, %.1f%% humidity\n", temperature, humidity);
}

bool Thermostat::isSensorDataValid() const {
    if (lastSensorUpdate == 0) return false;
    return (millis() - lastSensorUpdate) < sensorStaleTimeout;
}

int Thermostat::getFireplaceTemp() const {
    return ir->getCurrentTemp();
}

bool Thermostat::canChangeState() {
    if (lastStateChange == 0) return true;
    return (millis() - lastStateChange) >= minCycleTime;
}

void Thermostat::turnFireplaceOn() {
    if (!fireplaceOn) {
        Serial.println(">>> TURNING FIREPLACE ON <<<");
        ir->sendOn();
        delay(500);  // Wait for fireplace to fully power on

        ir->sendHeatOn();  // Turn on heat element
        delay(200);

        // Set fireplace temperature to target + offset
        int desiredFireplaceTemp = targetTemp + fireplaceOffset;
        ir->sendTemp(desiredFireplaceTemp);
        delay(200);

        // Turn off backlight (defaults to level 4, cycle to 0)
        // Light cycle: 4 -> 3 -> 2 -> 1 -> 0 (4 presses)
        ir->setLightLevel(4);  // Set internal state to match fireplace default
        for (int i = 0; i < 4; i++) {
            ir->sendLightToggle();
            delay(200);
        }
        // Light level now at 0 (off)

        fireplaceOn = true;
        heatingStartTime = millis();
        lastStateChange = millis();
        state = ThermostatState::HEATING;
    }
}

void Thermostat::turnFireplaceOff() {
    if (fireplaceOn) {
        Serial.println(">>> TURNING FIREPLACE OFF <<<");
        ir->sendOff();
        fireplaceOn = false;
        lastStateChange = millis();
        state = ThermostatState::SATISFIED;
    }
}

void Thermostat::evaluateState() {
    // If mode is OFF, ensure fireplace is off
    if (mode == ThermostatMode::OFF) {
        if (fireplaceOn && canChangeState()) {
            turnFireplaceOff();
        }
        state = ThermostatState::IDLE;
        return;
    }

    // Check if in cooldown period (after max runtime)
    if (inCooldown) {
        state = ThermostatState::COOLDOWN;
        return;  // Don't control fireplace during cooldown
    }

    // Check if in hold mode (manual override)
    if (holdActive) {
        state = ThermostatState::HOLD;
        return;  // Don't control fireplace during hold
    }

    // Check for stale sensor data
    if (!isSensorDataValid()) {
        Serial.println("Sensor data stale - going to IDLE");
        if (fireplaceOn && canChangeState()) {
            turnFireplaceOff();
        }
        state = ThermostatState::IDLE;
        return;
    }

    // Thermostat logic with hysteresis
    float lowerBound = targetTemp - hysteresis;
    float upperBound = targetTemp + hysteresis;

    if (!fireplaceOn) {
        // Currently OFF - turn ON if below lower bound
        if (currentTemp < lowerBound) {
            if (canChangeState()) {
                Serial.printf("Temp %.1f°F below threshold %.1f°F - heating needed\n",
                              currentTemp, lowerBound);
                turnFireplaceOn();
            } else {
                Serial.println("Would heat but in min cycle time");
            }
        } else {
            state = ThermostatState::SATISFIED;
        }
    } else {
        // Currently ON - turn OFF if above upper bound
        if (currentTemp > upperBound) {
            if (canChangeState()) {
                Serial.printf("Temp %.1f°F above threshold %.1f°F - stopping heat\n",
                              currentTemp, upperBound);
                turnFireplaceOff();
            } else {
                Serial.println("Would stop but in min cycle time");
            }
        } else {
            state = ThermostatState::HEATING;
        }
    }
}

void Thermostat::detectExternalRemote() {
    if (!isSensorDataValid()) return;

    unsigned long now = millis();
    if (now - lastTrendSample < TREND_SAMPLE_INTERVAL) return;
    lastTrendSample = now;

    // Skip first sample (no previous data)
    if (previousTemp == 0) {
        previousTemp = currentTemp;
        return;
    }

    float delta = currentTemp - previousTemp;
    previousTemp = currentTemp;

    // Determine trend direction
    int8_t newDirection = 0;
    if (delta > TREND_RISING_THRESHOLD) {
        newDirection = 1;  // Rising
    } else if (delta < TREND_FALLING_THRESHOLD) {
        newDirection = -1; // Falling
    }

    // Track consecutive samples
    if (newDirection == trendDirection && newDirection != 0) {
        consecutiveTrend++;
    } else {
        consecutiveTrend = (newDirection != 0) ? 1 : 0;
        trendDirection = newDirection;
    }

    // Check for state mismatch after enough consecutive samples
    if (consecutiveTrend >= TREND_SAMPLES_REQUIRED) {
        if (trendDirection == 1 && !fireplaceOn) {
            // Temp rising but we think fireplace is off - someone turned it on
            Serial.println(">>> DETECTED: Fireplace turned ON externally <<<");
            fireplaceOn = true;
            heatingStartTime = now;  // Track runtime for externally started heating
            consecutiveTrend = 0;
            // Auto-enter hold mode when external remote detected
            enterHold();
            Serial.println(">>> Auto-entering HOLD mode due to external remote <<<");
        } else if (trendDirection == -1 && fireplaceOn) {
            // Temp falling but we think fireplace is on - someone turned it off
            Serial.println(">>> DETECTED: Fireplace turned OFF externally <<<");
            fireplaceOn = false;
            heatingStartTime = 0;
            consecutiveTrend = 0;
            // Auto-enter hold mode when external remote detected
            enterHold();
            Serial.println(">>> Auto-entering HOLD mode due to external remote <<<");
        }
    }
}

void Thermostat::checkRuntimeLimit() {
    if (!fireplaceOn || heatingStartTime == 0) return;

    unsigned long now = millis();
    unsigned long runtime = now - heatingStartTime;

    if (runtime >= MAX_RUNTIME_MS) {
        Serial.println(">>> MAX RUNTIME REACHED - Turning heat OFF for safety <<<");
        Serial.printf("    Runtime was: %lu minutes\n", runtime / 60000);

        // Turn off heat but leave fireplace on (for lights/ambiance)
        ir->sendHeatOff();

        // Enter cooldown
        inCooldown = true;
        cooldownStartTime = now;
        heatingStartTime = 0;

        Serial.printf(">>> Entering %lu minute cooldown period <<<\n", (unsigned long)COOLDOWN_DURATION_MS / 60000);
    }
}

void Thermostat::checkCooldownComplete() {
    if (!inCooldown) return;

    unsigned long now = millis();
    if (now - cooldownStartTime >= COOLDOWN_DURATION_MS) {
        Serial.println(">>> Cooldown complete - resuming thermostat control <<<");
        inCooldown = false;
        cooldownStartTime = 0;
    }
}

void Thermostat::update() {
    unsigned long now = millis();

    // Check if hold mode has expired
    if (holdActive && (now - holdStartTime >= holdDuration)) {
        Serial.println(">>> Hold expired - resuming thermostat control <<<");
        holdActive = false;
    }

    // Check if cooldown has completed
    checkCooldownComplete();

    // Check runtime limit (safety)
    checkRuntimeLimit();

    // Detect external remote usage
    detectExternalRemote();

    // Main state machine
    evaluateState();

    // Handle pending settings save (debounced)
    if (settingsPendingSave && (now - lastSettingsChange >= SETTINGS_SAVE_DEBOUNCE_MS)) {
        saveSettings();
        settingsPendingSave = false;
    }
}

// Manual controls for web UI - these trigger hold mode
void Thermostat::manualOn() {
    Serial.println("Manual: Fireplace ON");
    ir->sendOn();
    fireplaceOn = true;
    heatingStartTime = millis();
    lastStateChange = millis();
    enterHold();  // Suspend auto-control
}

void Thermostat::manualOff() {
    Serial.println("Manual: Fireplace OFF");
    ir->sendOff();
    fireplaceOn = false;
    heatingStartTime = 0;
    lastStateChange = millis();
    enterHold();  // Suspend auto-control
}

void Thermostat::manualHeatOn() {
    Serial.println("Manual: Heat ON");
    ir->sendHeatOn();
    enterHold();  // Suspend auto-control
}

void Thermostat::manualHeatOff() {
    Serial.println("Manual: Heat OFF");
    ir->sendHeatOff();
    enterHold();  // Suspend auto-control
}

void Thermostat::manualHeatUp() {
    Serial.println("Manual: Heat UP");
    ir->sendHeatUp();
    // Don't enter hold for heat level adjustments
}

void Thermostat::manualHeatDown() {
    Serial.println("Manual: Heat DOWN");
    ir->sendHeatDown();
    // Don't enter hold for heat level adjustments
}

void Thermostat::manualLightToggle() {
    Serial.println("Manual: Light toggle");
    ir->sendLightToggle();
    // Don't enter hold for light adjustments
}

void Thermostat::manualTimerToggle() {
    Serial.println("Manual: Timer toggle");
    ir->sendTimerToggle();
    // Don't enter hold for timer adjustments
}

// ============================================================================
// Hold Mode Control
// ============================================================================
void Thermostat::enterHold(unsigned long durationMs) {
    holdActive = true;
    holdStartTime = millis();
    holdDuration = (durationMs > 0) ? durationMs : HOLD_DURATION_MS;
    Serial.printf(">>> Entering HOLD mode for %lu minutes <<<\n", holdDuration / 60000);
}

void Thermostat::exitHold() {
    if (holdActive) {
        Serial.println(">>> Exiting HOLD mode - resuming thermostat control <<<");
        holdActive = false;
    }
}

bool Thermostat::isInHold() const {
    return holdActive;
}

unsigned long Thermostat::getHoldRemaining() const {
    if (!holdActive) return 0;
    unsigned long elapsed = millis() - holdStartTime;
    if (elapsed >= holdDuration) return 0;
    return holdDuration - elapsed;
}

// ============================================================================
// Safety Status
// ============================================================================
bool Thermostat::isInCooldown() const {
    return inCooldown;
}

unsigned long Thermostat::getCooldownRemaining() const {
    if (!inCooldown) return 0;
    unsigned long elapsed = millis() - cooldownStartTime;
    if (elapsed >= COOLDOWN_DURATION_MS) return 0;
    return COOLDOWN_DURATION_MS - elapsed;
}

unsigned long Thermostat::getCurrentRuntime() const {
    if (!fireplaceOn || heatingStartTime == 0) return 0;
    return millis() - heatingStartTime;
}

void Thermostat::resetSafety() {
    Serial.println(">>> Safety reset - clearing cooldown and runtime <<<");
    inCooldown = false;
    cooldownStartTime = 0;
    heatingStartTime = 0;
}

// ============================================================================
// Settings Persistence
// ============================================================================
void Thermostat::markSettingsChanged() {
    lastSettingsChange = millis();
    settingsPendingSave = true;
}

void Thermostat::saveSettings() {
    if (!preferences.begin(PREFERENCES_NAMESPACE, false)) {
        Serial.println("ERROR: Failed to open preferences for writing");
        return;
    }

    bool success = true;
    size_t written;

    written = preferences.putFloat("targetTemp", targetTemp);
    if (written == 0) {
        Serial.println("ERROR: Failed to save targetTemp");
        success = false;
    }

    written = preferences.putFloat("hysteresis", hysteresis);
    if (written == 0) {
        Serial.println("ERROR: Failed to save hysteresis");
        success = false;
    }

    written = preferences.putUChar("mode", static_cast<uint8_t>(mode));
    if (written == 0) {
        Serial.println("ERROR: Failed to save mode");
        success = false;
    }

    preferences.end();

    if (success) {
        Serial.println("Settings saved to flash");
    } else {
        Serial.println("WARNING: Some settings failed to save");
    }
}

void Thermostat::loadSettings() {
    preferences.begin(PREFERENCES_NAMESPACE, true);  // Read-only
    if (preferences.isKey("targetTemp")) {
        targetTemp = preferences.getFloat("targetTemp", DEFAULT_TARGET_TEMP);
        hysteresis = preferences.getFloat("hysteresis", DEFAULT_HYSTERESIS);
        uint8_t modeVal = preferences.getUChar("mode", 0);
        mode = static_cast<ThermostatMode>(modeVal);
        fireplaceOffset = preferences.getInt("fpOffset", 4);  // Default +4°F
        Serial.println("Settings loaded from flash");
    } else {
        Serial.println("No saved settings found, using defaults");
    }
    preferences.end();
}

void Thermostat::setFireplaceOffset(int offset) {
    if (offset >= 2 && offset <= 10 && offset % 2 == 0) {  // Must be even
        fireplaceOffset = offset;

        // Save to EEPROM
        if (!preferences.begin(PREFERENCES_NAMESPACE, false)) {
            Serial.println("ERROR: Failed to open preferences for writing offset");
            return;
        }

        size_t written = preferences.putInt("fpOffset", offset);
        preferences.end();

        if (written > 0) {
            Serial.printf("Fireplace offset set to: +%d°F (saved)\n", offset);
        } else {
            Serial.println("ERROR: Failed to save fireplace offset");
        }
    } else {
        Serial.printf("Invalid offset %d (must be even, 2-10)\n", offset);
    }
}

uint8_t Thermostat::getLightLevel() const {
    return ir->getLightLevel();
}

uint8_t Thermostat::getTimerState() const {
    return ir->getTimerState();
}

const char* Thermostat::getTimerString() const {
    return ir->getTimerString();
}

const char* Thermostat::getModeString() const {
    switch (mode) {
        case ThermostatMode::OFF: return "OFF";
        case ThermostatMode::HEAT: return "HEAT";
        default: return "UNKNOWN";
    }
}

const char* Thermostat::getStateString() const {
    switch (state) {
        case ThermostatState::IDLE: return "IDLE";
        case ThermostatState::HEATING: return "HEATING";
        case ThermostatState::SATISFIED: return "SATISFIED";
        case ThermostatState::HOLD: return "HOLD";
        case ThermostatState::COOLDOWN: return "COOLDOWN";
        default: return "UNKNOWN";
    }
}
