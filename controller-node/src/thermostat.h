/*
 * Thermostat Logic Header
 * Implements temperature control with hysteresis
 */

#ifndef THERMOSTAT_H
#define THERMOSTAT_H

#include <Arduino.h>
#include "ir_controller.h"

enum class ThermostatMode {
    OFF,
    HEAT
};

enum class ThermostatState {
    IDLE,           // Mode is OFF or no valid sensor data
    HEATING,        // Fireplace is ON
    SATISFIED,      // Temperature reached, fireplace is OFF
    HOLD,           // Manual override - auto-control suspended
    COOLDOWN        // Post max-runtime cooldown period
};

class Thermostat {
public:
    Thermostat(IRController* irController);

    void begin();
    void update();

    // Setters
    void setTargetTemp(float temp);
    void setHysteresis(float hyst);
    void setMode(ThermostatMode mode);
    void updateSensorData(float temperature, float humidity);

    // Getters
    float getTargetTemp() const { return targetTemp; }
    float getHysteresis() const { return hysteresis; }
    float getCurrentTemp() const { return currentTemp; }
    float getCurrentHumidity() const { return currentHumidity; }
    ThermostatMode getMode() const { return mode; }
    ThermostatState getState() const { return state; }
    bool isFireplaceOn() const { return fireplaceOn; }
    bool isSensorDataValid() const;
    int getFireplaceOffset() const { return fireplaceOffset; }
    void setFireplaceOffset(int offset);
    int getFireplaceTemp() const;

    // Manual control (for web UI) - these trigger hold mode
    void manualOn();
    void manualOff();
    void manualHeatOn();
    void manualHeatOff();
    void manualHeatUp();
    void manualHeatDown();
    void manualLightToggle();
    void manualTimerToggle();

    // Hold mode control
    void enterHold(unsigned long durationMs = 0);  // 0 = use default
    void exitHold();
    bool isInHold() const;
    unsigned long getHoldRemaining() const;

    // Safety status
    bool isInCooldown() const;
    unsigned long getCooldownRemaining() const;
    unsigned long getCurrentRuntime() const;
    void resetSafety();  // Clear cooldown, reset runtime

    // Settings persistence
    void saveSettings();
    void loadSettings();

    // State getters for UI
    uint8_t getLightLevel() const;
    uint8_t getTimerState() const;
    const char* getTimerString() const;

    // State string helpers
    const char* getModeString() const;
    const char* getStateString() const;

private:
    IRController* ir;

    // Settings (persisted)
    float targetTemp;
    float hysteresis;
    ThermostatMode mode;
    int fireplaceOffset;  // Offset above thermostat target (default 4)

    // Current state
    ThermostatState state;
    float currentTemp;
    float currentHumidity;
    bool fireplaceOn;

    // Timing
    unsigned long lastSensorUpdate;
    unsigned long lastStateChange;
    unsigned long minCycleTime;
    unsigned long sensorStaleTimeout;

    // Hold mode
    bool holdActive;
    unsigned long holdStartTime;
    unsigned long holdDuration;

    // Runtime safety
    unsigned long heatingStartTime;      // When continuous heating began
    unsigned long cooldownStartTime;     // When cooldown period began
    bool inCooldown;

    // Settings persistence
    unsigned long lastSettingsChange;
    bool settingsPendingSave;

    // State machine
    void evaluateState();
    void turnFireplaceOn();
    void turnFireplaceOff();
    bool canChangeState();
    void checkRuntimeLimit();
    void checkCooldownComplete();

    // Temperature trend detection for external remote
    float previousTemp;
    unsigned long lastTrendSample;
    int8_t trendDirection;      // +1 rising, -1 falling, 0 stable
    int8_t consecutiveTrend;    // Count of consecutive samples in same direction

    void detectExternalRemote();
    void markSettingsChanged();
};

#endif // THERMOSTAT_H
