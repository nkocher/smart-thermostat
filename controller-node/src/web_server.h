/*
 * Web Server Header
 * Provides REST API and serves the web UI
 */

#ifndef WEB_SERVER_H
#define WEB_SERVER_H

#include <Arduino.h>
#include <ESPAsyncWebServer.h>
#include "thermostat.h"
#include "config.h"

class ThermostatWebServer {
public:
    ThermostatWebServer(Thermostat* thermostat, uint16_t port = 80);

    void begin();
    void setIRController(IRController* ir) { irController = ir; }

private:
    AsyncWebServer server;
    Thermostat* thermo;
    IRController* irController;

    // Route handlers
    void setupRoutes();

    // API endpoints
    void handleGetStatus(AsyncWebServerRequest* request);
    void handleSetTarget(AsyncWebServerRequest* request);
    void handleSetMode(AsyncWebServerRequest* request);
    void handleSetHysteresis(AsyncWebServerRequest* request);
    void handleSetOffset(AsyncWebServerRequest* request);

    // IR control endpoints
    void handleIROn(AsyncWebServerRequest* request);
    void handleIROff(AsyncWebServerRequest* request);
    void handleIRHeatOn(AsyncWebServerRequest* request);
    void handleIRHeatOff(AsyncWebServerRequest* request);
    void handleIRHeatUp(AsyncWebServerRequest* request);
    void handleIRHeatDown(AsyncWebServerRequest* request);
    void handleIRLightToggle(AsyncWebServerRequest* request);
    void handleIRTimerToggle(AsyncWebServerRequest* request);

    // Hold and safety endpoints
    void handleHoldEnter(AsyncWebServerRequest* request);
    void handleHoldExit(AsyncWebServerRequest* request);
    void handleSafetyReset(AsyncWebServerRequest* request);

    // Helpers
    String buildStatusJSON();
    bool isValidFloat(float value);
    bool isValidTemperature(float temp);
};

#endif // WEB_SERVER_H
