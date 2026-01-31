/*
 * Thermostat Sensor Node
 * ESP32 #1: DS18B20 temperature + DHT11 humidity with MQTT publishing
 *
 * Wiring:
 *   DS18B20 (HW-506) VCC  -> 3.3V
 *   DS18B20 (HW-506) DATA -> GPIO4 (HW-506 has onboard 4.7K pull-up)
 *   DS18B20 (HW-506) GND  -> GND
 *
 *   DHT11 VCC  -> GPIO17 (set HIGH = 3.3V power)
 *   DHT11 DATA -> GPIO16
 *   DHT11 GND  -> GND
 */

#include <Arduino.h>
#include <WiFi.h>
#include <WiFiManager.h>
#include <PubSubClient.h>
#include <DHT.h>
#include <OneWire.h>
#include <DallasTemperature.h>
#include <ArduinoJson.h>

#include "secrets.h"

// ============================================================================
// Configuration
// ============================================================================
// MQTT_SERVER is defined in secrets.h
#define MQTT_PORT 1883
#define MQTT_CLIENT_ID "thermostat-sensor"

#define DS18B20_PIN 4       // DS18B20 data
#define DHT_PIN 16          // DHT11 data (humidity only)
#define DHT_POWER_PIN 17    // GPIO17 powers the DHT11
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
OneWire oneWire(DS18B20_PIN);
DallasTemperature ds18b20(&oneWire);
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
    // Read temperature from DS18B20
    ds18b20.requestTemperatures();
    float tempC = ds18b20.getTempCByIndex(0);

    if (tempC != DEVICE_DISCONNECTED_C) {
        float tempF = tempC * 9.0 / 5.0 + 32.0;
        Serial.printf("[DS18B20] Temperature: %.1f°F (%.1f°C)\n", tempF, tempC);

        char tempStr[8];
        dtostrf(tempF, 4, 1, tempStr);
        mqtt.publish(TOPIC_TEMPERATURE, tempStr, true);
    } else {
        Serial.println("[DS18B20] Failed to read temperature!");
    }

    // Read humidity from DHT11
    float humidity = dht.readHumidity();

    if (!isnan(humidity)) {
        Serial.printf("[DHT11] Humidity: %.1f%%\n", humidity);

        char humStr[8];
        dtostrf(humidity, 4, 1, humStr);
        mqtt.publish(TOPIC_HUMIDITY, humStr, true);
    } else {
        Serial.println("[DHT11] Failed to read humidity!");
    }
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

    // Power DHT11 from GPIO17
    pinMode(DHT_POWER_PIN, OUTPUT);
    digitalWrite(DHT_POWER_PIN, HIGH);
    delay(100);  // Let DHT11 stabilize

    // Initialize DS18B20
    ds18b20.begin();
    ds18b20.setResolution(12);  // Best accuracy (±0.0625°C, ~750ms read)
    Serial.printf("DS18B20: %d device(s) found\n", ds18b20.getDeviceCount());

    // Initialize DHT11 (humidity only)
    dht.begin();
    Serial.println("DHT11 sensor initialized (humidity only)");

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
