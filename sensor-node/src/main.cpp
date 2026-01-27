/*
 * Thermostat Sensor Node
 * ESP32 #1: DHT11 temperature/humidity sensor with MQTT publishing
 *
 * Wiring:
 *   DHT11 VCC  -> 3.3V
 *   DHT11 DATA -> GPIO4 (+ 10K pull-up to 3.3V)
 *   DHT11 GND  -> GND
 */

#include <Arduino.h>
#include <WiFi.h>
#include <WiFiManager.h>
#include <PubSubClient.h>
#include <DHT.h>
#include <ArduinoJson.h>

#include "secrets.h"

// ============================================================================
// Configuration
// ============================================================================
// MQTT_SERVER is defined in secrets.h
#define MQTT_PORT 1883
#define MQTT_CLIENT_ID "thermostat-sensor"

#define DHT_PIN 4
#define DHT_TYPE DHT11

#define SENSOR_PUBLISH_INTERVAL 30000  // 30 seconds
#define WIFI_RECONNECT_INTERVAL 30000
#define MQTT_RECONNECT_INTERVAL 5000

// MQTT Topics
#define TOPIC_TEMPERATURE "thermostat/sensor/temperature"
#define TOPIC_HUMIDITY "thermostat/sensor/humidity"
#define TOPIC_STATUS "thermostat/sensor/status"

// ============================================================================
// Global Objects
// ============================================================================
WiFiClient espClient;
PubSubClient mqtt(espClient);
DHT dht(DHT_PIN, DHT_TYPE);

unsigned long lastPublishTime = 0;
unsigned long lastMqttReconnectAttempt = 0;

// ============================================================================
// WiFi Setup with WiFiManager
// ============================================================================
void setupWiFi() {
    Serial.println("Setting up WiFi...");

    WiFiManager wifiManager;

    // Reset settings for testing (uncomment if needed)
    // wifiManager.resetSettings();

    // Set timeout for config portal
    wifiManager.setConfigPortalTimeout(180);

    // Custom parameters could be added here for MQTT server config

    if (!wifiManager.autoConnect("ThermostatSensor-AP")) {
        Serial.println("Failed to connect and hit timeout");
        delay(3000);
        ESP.restart();
    }

    Serial.println("WiFi connected!");
    Serial.print("IP address: ");
    Serial.println(WiFi.localIP());
}

// ============================================================================
// MQTT Functions
// ============================================================================
void mqttCallback(char* topic, byte* payload, unsigned int length) {
    // Sensor node doesn't subscribe to any topics currently
    // But callback is required for PubSubClient
}

bool mqttReconnect() {
    Serial.print("Attempting MQTT connection...");

    if (mqtt.connect(MQTT_CLIENT_ID, MQTT_USER, MQTT_PASS)) {
        Serial.println("connected");

        // Publish online status
        mqtt.publish(TOPIC_STATUS, "online", true);

        return true;
    }

    Serial.print("failed, rc=");
    Serial.print(mqtt.state());
    Serial.println(" - will retry");
    return false;
}

void setupMQTT() {
    mqtt.setServer(MQTT_SERVER, MQTT_PORT);
    mqtt.setCallback(mqttCallback);
}

// ============================================================================
// Sensor Reading and Publishing
// ============================================================================
void readAndPublishSensor() {
    float humidity = dht.readHumidity();
    float tempC = dht.readTemperature();
    float tempF = dht.readTemperature(true);  // Fahrenheit

    if (isnan(humidity) || isnan(tempF)) {
        Serial.println("Failed to read from DHT sensor!");
        return;
    }

    Serial.printf("Temperature: %.1f°F (%.1f°C), Humidity: %.1f%%\n",
                  tempF, tempC, humidity);

    // Publish temperature (as string with 1 decimal place)
    char tempStr[8];
    dtostrf(tempF, 4, 1, tempStr);
    mqtt.publish(TOPIC_TEMPERATURE, tempStr, true);

    // Publish humidity
    char humStr[8];
    dtostrf(humidity, 4, 1, humStr);
    mqtt.publish(TOPIC_HUMIDITY, humStr, true);

    Serial.println("Published sensor data to MQTT");
}

// ============================================================================
// Setup and Loop
// ============================================================================
void setup() {
    Serial.begin(115200);
    delay(1000);

    Serial.println();
    Serial.println("========================================");
    Serial.println("Thermostat Sensor Node");
    Serial.println("========================================");

    // Initialize DHT sensor
    dht.begin();
    Serial.println("DHT11 sensor initialized");

    // Setup WiFi
    setupWiFi();

    // Setup MQTT
    setupMQTT();

    // Initial sensor read after a short delay
    delay(2000);
}

void loop() {
    unsigned long now = millis();

    // Check WiFi connection
    if (WiFi.status() != WL_CONNECTED) {
        Serial.println("WiFi disconnected, reconnecting...");
        WiFi.reconnect();
        delay(5000);
        return;
    }

    // Handle MQTT connection
    if (!mqtt.connected()) {
        if (now - lastMqttReconnectAttempt > MQTT_RECONNECT_INTERVAL) {
            lastMqttReconnectAttempt = now;
            if (mqttReconnect()) {
                lastMqttReconnectAttempt = 0;
            }
        }
    } else {
        mqtt.loop();
    }

    // Publish sensor data at regular intervals
    if (mqtt.connected() && (now - lastPublishTime >= SENSOR_PUBLISH_INTERVAL)) {
        lastPublishTime = now;
        readAndPublishSensor();
    }
}
