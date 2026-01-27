/*
 * Web Server Implementation
 * Provides REST API and serves the web UI
 */

#include "web_server.h"
#include <ArduinoJson.h>
#include <LittleFS.h>
#include <math.h>

ThermostatWebServer::ThermostatWebServer(Thermostat* thermostat, uint16_t port)
    : server(port), thermo(thermostat), irController(nullptr) {
}

// ============================================================================
// Input Validation Helpers
// ============================================================================
bool ThermostatWebServer::isValidFloat(float value) {
    return !isnan(value) && !isinf(value);
}

bool ThermostatWebServer::isValidTemperature(float temp) {
    return isValidFloat(temp) && temp >= MIN_VALID_TEMP && temp <= MAX_VALID_TEMP;
}

void ThermostatWebServer::begin() {
    // Initialize LittleFS for serving static files
    if (!LittleFS.begin(true)) {
        Serial.println("LittleFS mount failed!");
        return;
    }
    Serial.println("LittleFS mounted");

    setupRoutes();
    server.begin();
    Serial.println("Web server started on port 80");
}

void ThermostatWebServer::setupRoutes() {
    // API endpoints MUST be registered BEFORE serveStatic
    // Otherwise serveStatic catches /api/* and tries to serve files

    // API endpoints - Status
    server.on("/api/status", HTTP_GET, [this](AsyncWebServerRequest* request) {
        handleGetStatus(request);
    });

    // API endpoints - Thermostat control
    server.on("/api/target", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleSetTarget(request);
    });

    server.on("/api/mode", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleSetMode(request);
    });

    server.on("/api/hysteresis", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleSetHysteresis(request);
    });

    server.on("/api/offset", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleSetOffset(request);
    });

    // API endpoints - IR control (separate ON/OFF)
    server.on("/api/ir/on", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleIROn(request);
    });

    server.on("/api/ir/off", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleIROff(request);
    });

    server.on("/api/ir/heat/on", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleIRHeatOn(request);
    });

    server.on("/api/ir/heat/off", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleIRHeatOff(request);
    });

    server.on("/api/ir/heat/up", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleIRHeatUp(request);
    });

    server.on("/api/ir/heat/down", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleIRHeatDown(request);
    });

    server.on("/api/ir/light/toggle", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleIRLightToggle(request);
    });

    server.on("/api/ir/timer/toggle", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleIRTimerToggle(request);
    });

    // Hold mode endpoints
    server.on("/api/hold/enter", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleHoldEnter(request);
    });

    server.on("/api/hold/exit", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleHoldExit(request);
    });

    // Safety endpoint
    server.on("/api/safety/reset", HTTP_POST, [this](AsyncWebServerRequest* request) {
        handleSafetyReset(request);
    });

    // Serve static files AFTER API routes are registered
    server.serveStatic("/", LittleFS, "/").setDefaultFile("index.html");

    // 404 handler
    server.onNotFound([](AsyncWebServerRequest* request) {
        request->send(404, "text/plain", "Not found");
    });
}

String ThermostatWebServer::buildStatusJSON() {
    StaticJsonDocument<768> doc;

    doc["currentTemp"] = thermo->getCurrentTemp();
    doc["currentHumidity"] = thermo->getCurrentHumidity();
    doc["targetTemp"] = thermo->getTargetTemp();
    doc["hysteresis"] = thermo->getHysteresis();
    doc["fireplaceOffset"] = thermo->getFireplaceOffset();
    doc["fireplaceTemp"] = thermo->getFireplaceTemp();
    doc["mode"] = thermo->getModeString();
    doc["state"] = thermo->getStateString();
    doc["fireplaceOn"] = thermo->isFireplaceOn();
    doc["sensorValid"] = thermo->isSensorDataValid();
    doc["lightLevel"] = thermo->getLightLevel();
    doc["timerState"] = thermo->getTimerState();
    doc["timerString"] = thermo->getTimerString();

    // Hold mode status
    doc["holdActive"] = thermo->isInHold();
    doc["holdRemainingMs"] = thermo->getHoldRemaining();
    doc["holdRemainingMin"] = thermo->getHoldRemaining() / 60000;

    // Safety status
    doc["inCooldown"] = thermo->isInCooldown();
    doc["cooldownRemainingMs"] = thermo->getCooldownRemaining();
    doc["cooldownRemainingMin"] = thermo->getCooldownRemaining() / 60000;
    doc["runtimeMs"] = thermo->getCurrentRuntime();
    doc["runtimeMin"] = thermo->getCurrentRuntime() / 60000;

    String output;
    serializeJson(doc, output);
    return output;
}

void ThermostatWebServer::handleGetStatus(AsyncWebServerRequest* request) {
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleSetTarget(AsyncWebServerRequest* request) {
    float target;
    if (request->hasParam("value", true)) {
        target = request->getParam("value", true)->value().toFloat();
    } else if (request->hasParam("value")) {
        target = request->getParam("value")->value().toFloat();
    } else {
        request->send(400, "application/json", "{\"error\":\"Missing 'value' parameter\"}");
        return;
    }

    if (!isValidTemperature(target)) {
        request->send(400, "application/json", "{\"error\":\"Invalid temperature value\"}");
        return;
    }

    thermo->setTargetTemp(target);
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleSetMode(AsyncWebServerRequest* request) {
    String modeStr;

    if (request->hasParam("value", true)) {
        modeStr = request->getParam("value", true)->value();
    } else if (request->hasParam("value")) {
        modeStr = request->getParam("value")->value();
    } else {
        request->send(400, "application/json", "{\"error\":\"Missing 'value' parameter\"}");
        return;
    }

    modeStr.toUpperCase();

    if (modeStr == "HEAT") {
        thermo->setMode(ThermostatMode::HEAT);
    } else if (modeStr == "OFF") {
        thermo->setMode(ThermostatMode::OFF);
    } else {
        request->send(400, "application/json", "{\"error\":\"Invalid mode. Use 'HEAT' or 'OFF'\"}");
        return;
    }

    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleSetHysteresis(AsyncWebServerRequest* request) {
    float hyst;
    if (request->hasParam("value", true)) {
        hyst = request->getParam("value", true)->value().toFloat();
    } else if (request->hasParam("value")) {
        hyst = request->getParam("value")->value().toFloat();
    } else {
        request->send(400, "application/json", "{\"error\":\"Missing 'value' parameter\"}");
        return;
    }

    if (!isValidFloat(hyst) || hyst < 0.5 || hyst > 5.0) {
        request->send(400, "application/json", "{\"error\":\"Invalid hysteresis value (0.5-5.0)\"}");
        return;
    }

    thermo->setHysteresis(hyst);
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleSetOffset(AsyncWebServerRequest* request) {
    int offset;
    if (request->hasParam("value", true)) {
        offset = request->getParam("value", true)->value().toInt();
    } else if (request->hasParam("value")) {
        offset = request->getParam("value")->value().toInt();
    } else {
        request->send(400, "application/json", "{\"error\":\"Missing 'value' parameter\"}");
        return;
    }

    if (offset < 2 || offset > 10 || offset % 2 != 0) {
        request->send(400, "application/json", "{\"error\":\"Invalid offset value (2-10, even only)\"}");
        return;
    }

    thermo->setFireplaceOffset(offset);
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleIROn(AsyncWebServerRequest* request) {
    if (!irController) {
        request->send(500, "application/json", "{\"error\":\"IR controller not initialized\"}");
        return;
    }
    thermo->manualOn();
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleIROff(AsyncWebServerRequest* request) {
    if (!irController) {
        request->send(500, "application/json", "{\"error\":\"IR controller not initialized\"}");
        return;
    }
    thermo->manualOff();
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleIRHeatOn(AsyncWebServerRequest* request) {
    if (!irController) {
        request->send(500, "application/json", "{\"error\":\"IR controller not initialized\"}");
        return;
    }
    thermo->manualHeatOn();
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleIRHeatOff(AsyncWebServerRequest* request) {
    if (!irController) {
        request->send(500, "application/json", "{\"error\":\"IR controller not initialized\"}");
        return;
    }
    thermo->manualHeatOff();
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleIRHeatUp(AsyncWebServerRequest* request) {
    if (!irController) {
        request->send(500, "application/json", "{\"error\":\"IR controller not initialized\"}");
        return;
    }
    thermo->manualHeatUp();
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleIRHeatDown(AsyncWebServerRequest* request) {
    if (!irController) {
        request->send(500, "application/json", "{\"error\":\"IR controller not initialized\"}");
        return;
    }
    thermo->manualHeatDown();
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleIRLightToggle(AsyncWebServerRequest* request) {
    if (!irController) {
        request->send(500, "application/json", "{\"error\":\"IR controller not initialized\"}");
        return;
    }
    thermo->manualLightToggle();
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleIRTimerToggle(AsyncWebServerRequest* request) {
    if (!irController) {
        request->send(500, "application/json", "{\"error\":\"IR controller not initialized\"}");
        return;
    }
    thermo->manualTimerToggle();
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleHoldEnter(AsyncWebServerRequest* request) {
    unsigned long duration = 0;  // 0 = use default

    // Optional duration parameter (in minutes)
    if (request->hasParam("minutes", true)) {
        int minutes = request->getParam("minutes", true)->value().toInt();
        if (minutes > 0 && minutes <= MAX_HOLD_MINUTES) {
            duration = minutes * 60000UL;
        }
    } else if (request->hasParam("minutes")) {
        int minutes = request->getParam("minutes")->value().toInt();
        if (minutes > 0 && minutes <= MAX_HOLD_MINUTES) {
            duration = minutes * 60000UL;
        }
    }

    thermo->enterHold(duration);
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleHoldExit(AsyncWebServerRequest* request) {
    thermo->exitHold();
    request->send(200, "application/json", buildStatusJSON());
}

void ThermostatWebServer::handleSafetyReset(AsyncWebServerRequest* request) {
    thermo->resetSafety();
    request->send(200, "application/json", buildStatusJSON());
}
