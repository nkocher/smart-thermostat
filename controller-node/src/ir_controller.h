/*
 * IR Controller Header
 * Handles IR transmission to control the fireplace using raw timing data
 * Includes state tracking for light (0-4) and timer (0-10 for off, 0.5hr, 1-9hr)
 */

#ifndef IR_CONTROLLER_H
#define IR_CONTROLLER_H

#include <Arduino.h>
#include <IRremoteESP8266.h>
#include <IRsend.h>

class IRController {
public:
    IRController(uint16_t sendPin);

    void begin();

    // Fireplace power control (separate ON/OFF commands)
    void sendOn();
    void sendOff();

    // Heat control
    void sendHeatOn();
    void sendHeatOff();
    void sendHeatUp();
    void sendHeatDown();

    // Temperature control (state-dependent codes like light)
    void sendTemp(int temp);  // Send UP/DOWN codes to reach target temp (60-80°F, even only)
    int getCurrentTemp() const { return currentTemp; }
    void setCurrentTemp(int temp);  // For state sync

    // Light control with state tracking
    // Light cycles: off(0) -> 4 -> 3 -> 2 -> 1 -> off(0)
    void sendLightToggle();  // Cycle through light levels
    uint8_t getLightLevel() const { return lightLevel; }
    void setLightLevel(uint8_t level);  // For state sync

    // Timer control with state tracking
    // Timer cycles: off(0) -> 0.5hr(1) -> 1hr(2) -> 2hr(3) -> ... -> 9hr(10) -> off(0)
    void sendTimerToggle();  // Cycle through timer values
    uint8_t getTimerState() const { return timerState; }
    void setTimerState(uint8_t state);  // For state sync

    // Get timer display string (e.g., "OFF", "0.5hr", "1hr", etc.)
    const char* getTimerString() const;

    // Generic raw send (for testing)
    void sendRaw(const uint16_t* data, uint16_t len);

private:
    IRsend irsend;

    // State tracking
    uint8_t lightLevel;  // 0=off, 1-4=brightness level
    uint8_t timerState;  // 0=off, 1=0.5hr, 2=1hr, 3=2hr, ..., 10=9hr
    int currentTemp;     // Current fireplace temperature (60-80°F, even only)

    // Timing
    unsigned long lastSendTime;
    static const unsigned long MIN_SEND_INTERVAL = 300; // ms between sends

    bool canSend();

    // Internal send methods for state-aware codes
    void sendLightCode();
    void sendTimerCode();
    void sendTempUpCode();   // Send UP based on current temp
    void sendTempDownCode(); // Send DOWN based on current temp
};

#endif // IR_CONTROLLER_H
