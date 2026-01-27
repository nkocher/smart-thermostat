/*
 * Thermostat Controller Node - Main
 * ESP32-S3: IR control, thermostat logic, and web UI
 *
 * Wiring (ESP32-S3-N16R8):
 *   IR Receiver OUT -> GPIO14
 *   IR Receiver VCC -> 3.3V
 *   IR Receiver GND -> GND
 *   IR LED Circuit (using 5V for better range):
 *     GPIO4 -> 1kΩ -> 2N2222 base
 *     5V -> 56Ω (or 68Ω) -> IR LED anode
 *     IR LED cathode -> 2N2222 collector
 *     2N2222 emitter -> GND
 *
 * LED Status (GPIO48 on ESP32-S3, GPIO2 on original ESP32):
 *   - Fast blink (100ms): Connecting to WiFi
 *   - Slow blink (1s): WiFi connected, MQTT disconnected
 *   - Solid ON: Fully connected
 *   - Off: Error state
 */

#include <Arduino.h>
#include <WiFi.h>
#include <WiFiManager.h>
#include <PubSubClient.h>
#include <ArduinoJson.h>
#include <ArduinoOTA.h>
#include <esp_task_wdt.h>
#include <math.h>

#include "config.h"
#include "ir_controller.h"
#include "thermostat.h"
#include "web_server.h"

// ============================================================================
// Status LED Configuration
// ============================================================================
// ESP32-S3-DevKitC-1 uses GPIO48 for RGB LED; original ESP32 uses GPIO2
#ifdef CONFIG_IDF_TARGET_ESP32S3
#define STATUS_LED_PIN 48
#else
#define STATUS_LED_PIN 2
#endif

// ============================================================================
// Static IP Configuration (comment out to use DHCP)
// IP addresses are configured in secrets.h
// ============================================================================
#define USE_STATIC_IP
#ifdef USE_STATIC_IP
IPAddress staticIP(STATIC_IP_ADDR);
IPAddress gateway(STATIC_IP_GATEWAY);
IPAddress subnet(STATIC_IP_SUBNET);
IPAddress dns(STATIC_IP_DNS);
#endif

// ============================================================================
// Global Objects
// ============================================================================
WiFiClient espClient;
PubSubClient mqtt(espClient);

IRController irController(IR_SEND_PIN);
Thermostat thermostat(&irController);
ThermostatWebServer webServer(&thermostat);

// Timing
unsigned long lastMqttReconnectAttempt = 0;
unsigned long lastStatePublish = 0;
unsigned long lastLedToggle = 0;
bool ledState = false;

// WiFi event tracking
bool wifiWasConnected = false;

// ============================================================================
// LED Status Functions
// ============================================================================
void updateStatusLED(int blinkInterval) {
    unsigned long now = millis();
    if (blinkInterval == 0) {
        // Solid on
        digitalWrite(STATUS_LED_PIN, HIGH);
        ledState = true;
    } else if (blinkInterval < 0) {
        // Off
        digitalWrite(STATUS_LED_PIN, LOW);
        ledState = false;
    } else {
        // Blink at specified interval
        if (now - lastLedToggle >= (unsigned long)blinkInterval) {
            lastLedToggle = now;
            ledState = !ledState;
            digitalWrite(STATUS_LED_PIN, ledState ? HIGH : LOW);
        }
    }
}

// ============================================================================
// WiFi Setup with WiFiManager
// ============================================================================
void setupWiFi() {
    Serial.println("Setting up WiFi...");

    // WiFi stability settings
    WiFi.mode(WIFI_STA);
    WiFi.setSleep(false);  // Disable WiFi power saving for stability
    WiFi.setAutoReconnect(true);

#ifdef USE_STATIC_IP
    WiFi.config(staticIP, gateway, subnet, dns);
    Serial.println("Using static IP configuration");
#endif

    WiFiManager wifiManager;

    // Reset settings for testing (uncomment if needed)
    // wifiManager.resetSettings();

    wifiManager.setConfigPortalTimeout(180);
    wifiManager.setConnectTimeout(30);  // 30 second timeout per connection attempt
    wifiManager.setConnectRetries(3);   // Retry 3 times before giving up

    // Blink fast while connecting
    for (int i = 0; i < 20; i++) {
        digitalWrite(STATUS_LED_PIN, i % 2);
        delay(100);
    }

    if (!wifiManager.autoConnect("ThermostatController-AP")) {
        Serial.println("Failed to connect, restarting...");
        digitalWrite(STATUS_LED_PIN, LOW);
        delay(3000);
        ESP.restart();
    }

    Serial.println("WiFi connected!");
    Serial.print("IP address: ");
    Serial.println(WiFi.localIP());
    Serial.printf("Signal strength: %d dBm\n", WiFi.RSSI());
    Serial.printf("Channel: %d\n", WiFi.channel());

    // Solid LED = WiFi connected
    digitalWrite(STATUS_LED_PIN, HIGH);

    // Give WiFiManager's web server time to fully release port 80
    delay(1000);
}

// ============================================================================
// Input Validation Helper
// ============================================================================
bool isValidFloat(float value) {
    return !isnan(value) && !isinf(value);
}

bool isValidTemperature(float temp) {
    return isValidFloat(temp) && temp >= MIN_VALID_TEMP && temp <= MAX_VALID_TEMP;
}

// ============================================================================
// MQTT Callback
// ============================================================================
void mqttCallback(char* topic, byte* payload, unsigned int length) {
    // Guard against oversized messages
    if (length >= MAX_MQTT_MSG) {
        Serial.printf("MQTT message too large (%u bytes), ignoring\n", length);
        return;
    }

    // Use fixed-size buffer instead of VLA
    char message[MAX_MQTT_MSG];
    memcpy(message, payload, length);
    message[length] = '\0';

    Serial.printf("MQTT [%s]: %s\n", topic, message);

    // Handle sensor temperature updates
    if (strcmp(topic, TOPIC_SENSOR_TEMP) == 0) {
        float temp = atof(message);
        if (!isValidTemperature(temp)) {
            Serial.printf("Invalid temperature value: %s\n", message);
            return;
        }
        thermostat.updateSensorData(temp, thermostat.getCurrentHumidity());
    }
    // Handle sensor humidity updates
    else if (strcmp(topic, TOPIC_SENSOR_HUMIDITY) == 0) {
        float humidity = atof(message);
        if (!isValidFloat(humidity) || humidity < 0 || humidity > 100) {
            Serial.printf("Invalid humidity value: %s\n", message);
            return;
        }
        thermostat.updateSensorData(thermostat.getCurrentTemp(), humidity);
    }
    // Handle power command (accepts "on" or "off")
    else if (strcmp(topic, TOPIC_CMD_POWER) == 0) {
        String cmd = String(message);
        cmd.toLowerCase();
        if (cmd == "on") {
            thermostat.manualOn();
        } else if (cmd == "off") {
            thermostat.manualOff();
        }
    }
    // Handle target temperature command
    else if (strcmp(topic, TOPIC_CMD_TARGET) == 0) {
        float target = atof(message);
        if (!isValidTemperature(target)) {
            Serial.printf("Invalid target temperature: %s\n", message);
            return;
        }
        thermostat.setTargetTemp(target);
    }
    // Handle mode command
    else if (strcmp(topic, TOPIC_CMD_MODE) == 0) {
        String modeStr = String(message);
        modeStr.toUpperCase();
        if (modeStr == "HEAT") {
            thermostat.setMode(ThermostatMode::HEAT);
        } else if (modeStr == "OFF") {
            thermostat.setMode(ThermostatMode::OFF);
        }
    }
    // Handle hold command (accepts "on", "off", or minutes as integer)
    else if (strcmp(topic, TOPIC_CMD_HOLD) == 0) {
        String cmd = String(message);
        cmd.toLowerCase();
        if (cmd == "on" || cmd == "enter") {
            thermostat.enterHold();
        } else if (cmd == "off" || cmd == "exit") {
            thermostat.exitHold();
        } else {
            // Try to parse as duration in minutes
            int minutes = cmd.toInt();
            if (minutes > 0 && minutes <= MAX_HOLD_MINUTES) {
                thermostat.enterHold(minutes * 60000UL);
            } else if (minutes > MAX_HOLD_MINUTES) {
                Serial.printf("Hold duration %d exceeds max %d minutes\n", minutes, MAX_HOLD_MINUTES);
            }
        }
    }
}

// ============================================================================
// MQTT Functions
// ============================================================================
bool mqttReconnect() {
    Serial.print("Attempting MQTT connection...");

    if (mqtt.connect(MQTT_CLIENT_ID, MQTT_USER, MQTT_PASS)) {
        Serial.println("connected");

        // Subscribe to sensor data
        mqtt.subscribe(TOPIC_SENSOR_TEMP);
        mqtt.subscribe(TOPIC_SENSOR_HUMIDITY);

        // Subscribe to commands
        mqtt.subscribe(TOPIC_CMD_POWER);
        mqtt.subscribe(TOPIC_CMD_TARGET);
        mqtt.subscribe(TOPIC_CMD_MODE);
        mqtt.subscribe(TOPIC_CMD_HOLD);

        Serial.println("Subscribed to topics");
        return true;
    }

    Serial.print("failed, rc=");
    Serial.print(mqtt.state());
    Serial.println(" - will retry");
    return false;
}

void publishState() {
    StaticJsonDocument<384> doc;

    doc["temp"] = thermostat.getCurrentTemp();
    doc["humidity"] = thermostat.getCurrentHumidity();
    doc["target"] = thermostat.getTargetTemp();
    doc["mode"] = thermostat.getModeString();
    doc["state"] = thermostat.getStateString();
    doc["fireplace"] = thermostat.isFireplaceOn();

    // Hold and safety status
    doc["holdActive"] = thermostat.isInHold();
    doc["holdRemainingMin"] = thermostat.getHoldRemaining() / 60000;
    doc["inCooldown"] = thermostat.isInCooldown();
    doc["cooldownRemainingMin"] = thermostat.getCooldownRemaining() / 60000;
    doc["runtimeMin"] = thermostat.getCurrentRuntime() / 60000;

    String output;
    serializeJson(doc, output);

    mqtt.publish(TOPIC_CONTROLLER_STATE, output.c_str(), true);
}

void setupMQTT() {
    mqtt.setServer(MQTT_SERVER, MQTT_PORT);
    mqtt.setCallback(mqttCallback);
    mqtt.setBufferSize(512);
}

// ============================================================================
// Setup and Loop
// ============================================================================
void setup() {
    Serial.begin(115200);
    delay(1000);

    // Initialize status LED
    pinMode(STATUS_LED_PIN, OUTPUT);
    digitalWrite(STATUS_LED_PIN, LOW);

    Serial.println();
    Serial.println("========================================");
    Serial.println("Thermostat Controller Node");
    Serial.println("========================================");

    // Initialize IR controller
    irController.begin();

    // Initialize thermostat
    thermostat.begin();

    // Setup WiFi (LED will blink during connection)
    setupWiFi();

    // Setup OTA Updates with authentication
    ArduinoOTA.setHostname("thermostat-controller");
    ArduinoOTA.setPassword(OTA_PASSWORD);
    ArduinoOTA.onStart([]() {
        Serial.println("OTA Update starting...");
    });
    ArduinoOTA.onEnd([]() {
        Serial.println("\nOTA Update complete!");
    });
    ArduinoOTA.onProgress([](unsigned int progress, unsigned int total) {
        if (total > 0) {
            Serial.printf("Progress: %u%%\r", (progress * 100) / total);
        }
    });
    ArduinoOTA.onError([](ota_error_t error) {
        Serial.printf("Error[%u]: ", error);
        if (error == OTA_AUTH_ERROR) Serial.println("Auth Failed");
        else if (error == OTA_BEGIN_ERROR) Serial.println("Begin Failed");
        else if (error == OTA_CONNECT_ERROR) Serial.println("Connect Failed");
        else if (error == OTA_RECEIVE_ERROR) Serial.println("Receive Failed");
        else if (error == OTA_END_ERROR) Serial.println("End Failed");
    });
    ArduinoOTA.begin();
    Serial.println("OTA ready (password protected)");

    // Setup MQTT
    setupMQTT();

    // Setup web server
    webServer.setIRController(&irController);
    webServer.begin();

    // Initialize hardware watchdog (30 second timeout)
    esp_task_wdt_init(30, true);
    esp_task_wdt_add(NULL);
    Serial.println("Watchdog timer initialized (30s)");

    Serial.println("Setup complete!");
    Serial.println("========================================");
}

void loop() {
    unsigned long now = millis();
    static unsigned long lastWifiCheck = 0;
    static unsigned long lastRssiPrint = 0;
    static int wifiRetryCount = 0;

    // Reset watchdog timer at start of each loop
    esp_task_wdt_reset();

    // ALWAYS update thermostat logic - even during WiFi/MQTT issues
    // This ensures local safety features continue to work
    thermostat.update();

    // Check WiFi connection (non-blocking)
    if (WiFi.status() != WL_CONNECTED) {
        // Fast blink while WiFi is disconnected
        updateStatusLED(100);

        if (wifiWasConnected) {
            Serial.println("WiFi disconnected - continuing local thermostat operation");
            wifiWasConnected = false;
        }

        if (now - lastWifiCheck > 10000) {  // Only try every 10 seconds
            lastWifiCheck = now;
            wifiRetryCount++;
            Serial.printf("WiFi reconnect attempt %d...\n", wifiRetryCount);

            WiFi.disconnect(true);
            delay(100);  // Short delay, not blocking
            WiFi.begin();  // Reconnect to saved network

            // If we've failed many times, restart the ESP32
            if (wifiRetryCount >= 30) {  // 5 minutes of failures
                Serial.println("Too many WiFi failures, restarting...");
                delay(1000);
                ESP.restart();
            }
        }
        // Don't return - continue with thermostat.update() above
    } else {
        // WiFi is connected
        if (!wifiWasConnected) {
            Serial.println("WiFi reconnected!");
            Serial.printf("IP: %s, RSSI: %d dBm\n",
                          WiFi.localIP().toString().c_str(), WiFi.RSSI());
            wifiWasConnected = true;
        }
        wifiRetryCount = 0;

        // Handle OTA updates
        ArduinoOTA.handle();

        // Print WiFi signal strength every 30 seconds
        if (now - lastRssiPrint > 30000) {
            lastRssiPrint = now;
            Serial.printf("WiFi RSSI: %d dBm (IP: %s, Channel: %d)\n",
                          WiFi.RSSI(), WiFi.localIP().toString().c_str(), WiFi.channel());
        }

        // Handle MQTT connection
        if (!mqtt.connected()) {
            // Slow blink when WiFi OK but MQTT disconnected
            updateStatusLED(1000);

            if (now - lastMqttReconnectAttempt > 5000) {
                lastMqttReconnectAttempt = now;
                if (mqttReconnect()) {
                    lastMqttReconnectAttempt = 0;
                }
            }
        } else {
            // Solid LED = fully connected
            updateStatusLED(0);
            mqtt.loop();

            // Publish state periodically
            if (now - lastStatePublish >= STATE_PUBLISH_INTERVAL) {
                lastStatePublish = now;
                publishState();
            }
        }
    }
}
