/*
 * Web-Based IR Code Learner
 *
 * A web UI tool for capturing and testing IR codes from the SimpliFire remote.
 *
 * Features:
 *   - WiFi setup via captive portal (WiFiManager)
 *   - Web UI with button grid for each remote button
 *   - Capture workflow: Click button -> "Waiting..." -> Press remote -> Show code
 *   - Test button to verify captured codes
 *   - Export generates config.h code
 *
 * Wiring:
 *   IR Receiver OUT -> GPIO14
 *   IR Receiver VCC -> 3.3V
 *   IR Receiver GND -> GND
 *   IR LED (via 2N2222 transistor) -> GPIO4
 */

#include <Arduino.h>
#include <WiFi.h>
#include <WiFiManager.h>
#include <WebServer.h>
#include <LittleFS.h>
#include <ArduinoJson.h>
#include <IRremoteESP8266.h>
#include <IRrecv.h>
#include <IRsend.h>
#include <IRutils.h>

// Pin definitions
#define IR_RECV_PIN 14
#define IR_SEND_PIN 4
#define IR_SEND_FREQ 36  // 36kHz for SimpliFire

// Maximum raw data length we'll store (existing codes are ~99 values)
#define MAX_RAW_LEN 120

// IR objects
IRrecv irrecv(IR_RECV_PIN);
IRsend irsend(IR_SEND_PIN);
decode_results results;

// Web server (synchronous - avoids conflicts with IR interrupts)
WebServer server(80);

// Button definitions - expanded for all possible states
const char* buttonNames[] = {
    // Power (2 states)
    "power_on",           // Press when fireplace is OFF -> turns ON
    "power_off",          // Press when fireplace is ON -> turns OFF

    // Light levels (5 states - capture at each level)
    "light_from_4",       // Press when at level 4 -> goes to 3
    "light_from_3",       // Press when at level 3 -> goes to 2
    "light_from_2",       // Press when at level 2 -> goes to 1
    "light_from_1",       // Press when at level 1 -> goes to off
    "light_from_off",     // Press when light is off -> goes to 4

    // Heat toggle (2 states)
    "heat_on",            // Press when heat is OFF -> turns ON
    "heat_off",           // Press when heat is ON -> turns OFF

    // Heat level adjustment
    "heat_up",            // Increase heat level
    "heat_down",          // Decrease heat level

    // Timer (11 states)
    "timer_from_off",     // Press when timer off -> 0.5hr
    "timer_from_0.5",     // Press when at 0.5hr -> 1hr
    "timer_from_1",       // Press when at 1hr -> 2hr
    "timer_from_2",       // Press when at 2hr -> 3hr
    "timer_from_3",       // Press when at 3hr -> 4hr
    "timer_from_4",       // Press when at 4hr -> 5hr
    "timer_from_5",       // Press when at 5hr -> 6hr
    "timer_from_6",       // Press when at 6hr -> 7hr
    "timer_from_7",       // Press when at 7hr -> 8hr
    "timer_from_8",       // Press when at 8hr -> 9hr
    "timer_from_9"        // Press when at 9hr -> off
};
const int NUM_BUTTONS = 22;

// Storage for captured codes
struct CapturedCode {
    bool captured;
    uint16_t rawData[MAX_RAW_LEN];
    uint16_t rawLen;
    char protocol[16];  // Fixed size instead of String to avoid heap fragmentation
    uint64_t value;
    uint16_t bits;
};

CapturedCode capturedCodes[NUM_BUTTONS];

// Capture state
volatile bool isCapturing = false;
volatile int captureButtonIndex = -1;
volatile bool newCodeReceived = false;

// Temporary capture for dynamic button names (e.g., temp_60, temp_62, etc.)
CapturedCode tempCapture;
bool isTempCapture = false;


// Forward declarations
void setupWiFi();
void setupWebServer();
void handleIRCapture();
String generateConfigH();
int getButtonIndex(const String& name);

void setup() {
    Serial.begin(115200);
    delay(1000);

    Serial.println();
    Serial.println("========================================");
    Serial.println("Web-Based IR Code Learner");
    Serial.println("========================================");

    // Zero-initialize the entire array
    memset(capturedCodes, 0, sizeof(capturedCodes));

    Serial.printf("Free heap: %d bytes\n", ESP.getFreeHeap());
    Serial.printf("Struct size: %d bytes x %d = %d bytes\n",
                  sizeof(CapturedCode), NUM_BUTTONS, sizeof(CapturedCode) * NUM_BUTTONS);

    // Initialize LittleFS
    if (!LittleFS.begin(true)) {
        Serial.println("ERROR: Failed to mount LittleFS");
    } else {
        Serial.println("LittleFS mounted successfully");
    }

    // Initialize IR - receiver will be enabled only during capture
    irsend.begin();
    Serial.println("IR initialized (receiver enables only during capture)");

    // Setup WiFi
    setupWiFi();

    // Wait 1 second before starting web server (port 80 conflict with WiFiManager)
    delay(1000);

    // Setup web server
    setupWebServer();

    Serial.println();
    Serial.println("Ready! Open http://" + WiFi.localIP().toString() + " in your browser");
    Serial.println("========================================");
}

void loop() {
    server.handleClient();  // Handle web requests
    handleIRCapture();
    delay(1);
}

void setupWiFi() {
    WiFiManager wm;

    // Reset settings for testing (uncomment if needed)
    // wm.resetSettings();

    wm.setConfigPortalTimeout(180);  // 3 minute timeout

    Serial.println("Starting WiFiManager...");

    if (!wm.autoConnect("IR-Learner-Setup")) {
        Serial.println("Failed to connect, restarting...");
        delay(3000);
        ESP.restart();
    }

    Serial.println("WiFi connected!");
    Serial.print("IP Address: ");
    Serial.println(WiFi.localIP());

    // Disable WiFi sleep for stability
    WiFi.setSleep(false);
}

// Handler functions for synchronous WebServer
void handleStatus() {
    String response = "{\"capturing\":";
    response += isCapturing ? "true" : "false";
    response += ",\"captureButton\":\"";
    if (captureButtonIndex >= 0 && captureButtonIndex < NUM_BUTTONS) {
        response += buttonNames[captureButtonIndex];
    }
    response += "\",\"newCode\":";
    response += newCodeReceived ? "true" : "false";
    response += "}";
    server.send(200, "application/json", response);
}

void handleCaptureStart() {
    if (!server.hasArg("button")) {
        server.send(400, "application/json", "{\"error\":\"Missing button parameter\"}");
        return;
    }

    String buttonName = server.arg("button");
    int idx = getButtonIndex(buttonName);

    if (idx < 0) {
        // Button not in predefined list - use temporary capture for dynamic names
        Serial.printf("Starting TEMP capture for dynamic button: %s\n", buttonName.c_str());
        isTempCapture = true;
        captureButtonIndex = -1;
        memset(&tempCapture, 0, sizeof(tempCapture));
    } else {
        // Standard capture for predefined buttons
        Serial.printf("Starting capture for button: %s\n", buttonName.c_str());
        isTempCapture = false;
        captureButtonIndex = idx;
    }

    newCodeReceived = false;
    isCapturing = true;
    irrecv.enableIRIn();

    server.send(200, "application/json", "{\"status\":\"capturing\"}");
}

void handleCaptureStop() {
    isCapturing = false;
    captureButtonIndex = -1;
    newCodeReceived = false;
    isTempCapture = false;
    irrecv.disableIRIn();

    Serial.println("Capture stopped");
    server.send(200, "application/json", "{\"status\":\"stopped\"}");
}

void handleCodes() {
    Serial.println("handleCodes() called");
    int capturedCount = 0;
    for (int i = 0; i < NUM_BUTTONS; i++) {
        if (capturedCodes[i].captured) capturedCount++;
    }
    Serial.printf("  Total captured: %d of %d\n", capturedCount, NUM_BUTTONS);

    String response = "{\"codes\":[";
    for (int i = 0; i < NUM_BUTTONS; i++) {
        if (i > 0) response += ",";
        response += "{\"name\":\"";
        response += buttonNames[i];
        response += "\",\"captured\":";
        response += capturedCodes[i].captured ? "true" : "false";
        if (capturedCodes[i].captured) {
            response += ",\"protocol\":\"";
            response += capturedCodes[i].protocol[0] ? capturedCodes[i].protocol : "UNKNOWN";
            response += "\",\"bits\":";
            response += capturedCodes[i].bits;
            response += ",\"rawLen\":";
            response += capturedCodes[i].rawLen;
        }
        response += "}";
    }
    response += "]}";
    server.send(200, "application/json", response);
}

void handleCodesRaw() {
    if (!server.hasArg("button")) {
        server.send(400, "application/json", "{\"error\":\"Missing button parameter\"}");
        return;
    }

    String buttonName = server.arg("button");
    int idx = getButtonIndex(buttonName);

    CapturedCode* code = nullptr;

    if (idx < 0) {
        // Check if this is a temp capture (dynamic button name)
        if (tempCapture.captured) {
            code = &tempCapture;
        } else {
            server.send(404, "application/json", "{\"error\":\"No code captured for this button\"}");
            return;
        }
    } else if (idx >= NUM_BUTTONS) {
        server.send(400, "application/json", "{\"error\":\"Invalid button index\"}");
        return;
    } else {
        if (!capturedCodes[idx].captured) {
            server.send(404, "application/json", "{\"error\":\"No code captured\"}");
            return;
        }
        code = &capturedCodes[idx];
    }

    String response = "{\"name\":\"";
    response += buttonName;
    response += "\",\"rawLen\":";
    response += code->rawLen;
    response += ",\"rawData\":[";
    for (int i = 0; i < code->rawLen && i < MAX_RAW_LEN; i++) {
        if (i > 0) response += ",";
        response += code->rawData[i];
    }
    response += "]}";
    server.send(200, "application/json", response);
}

void handleTest() {
    if (!server.hasArg("button")) {
        server.send(400, "application/json", "{\"error\":\"Missing button parameter\"}");
        return;
    }

    String buttonName = server.arg("button");
    int idx = getButtonIndex(buttonName);

    if (idx < 0 || idx >= NUM_BUTTONS || !capturedCodes[idx].captured) {
        server.send(404, "application/json", "{\"error\":\"No code captured\"}");
        return;
    }

    Serial.printf("Testing IR code for: %s\n", buttonName.c_str());
    for (int i = 0; i < 3; i++) {
        irsend.sendRaw(capturedCodes[idx].rawData, capturedCodes[idx].rawLen, IR_SEND_FREQ);
        delay(50);
    }
    server.send(200, "application/json", "{\"status\":\"sent\"}");
}

void handleExport() {
    Serial.println("handleExport() called");
    int capturedCount = 0;
    for (int i = 0; i < NUM_BUTTONS; i++) {
        if (capturedCodes[i].captured) {
            capturedCount++;
            Serial.printf("  [%d] %s: rawLen=%d\n", i, buttonNames[i], capturedCodes[i].rawLen);
        }
    }
    Serial.printf("  Total captured: %d\n", capturedCount);

    String configH = generateConfigH();
    server.send(200, "text/plain", configH);
}

void handleCodesClear() {
    if (!server.hasArg("button")) {
        server.send(400, "application/json", "{\"error\":\"Missing button parameter\"}");
        return;
    }

    String buttonName = server.arg("button");
    int idx = getButtonIndex(buttonName);

    if (idx < 0) {
        server.send(400, "application/json", "{\"error\":\"Invalid button name\"}");
        return;
    }

    capturedCodes[idx].captured = false;
    capturedCodes[idx].rawLen = 0;
    Serial.printf("Cleared: %s\n", buttonName.c_str());
    server.send(200, "application/json", "{\"status\":\"cleared\"}");
}

void handleNotFound() {
    // Try to serve from LittleFS
    String path = server.uri();
    if (path.endsWith("/")) path += "index.html";

    String contentType = "text/plain";
    if (path.endsWith(".html")) contentType = "text/html";
    else if (path.endsWith(".css")) contentType = "text/css";
    else if (path.endsWith(".js")) contentType = "application/javascript";

    if (LittleFS.exists(path)) {
        File file = LittleFS.open(path, "r");
        server.streamFile(file, contentType);
        file.close();
    } else {
        server.send(404, "text/plain", "Not Found");
    }
}

void setupWebServer() {
    // API routes
    server.on("/api/status", HTTP_GET, handleStatus);
    server.on("/api/capture/start", HTTP_POST, handleCaptureStart);
    server.on("/api/capture/stop", HTTP_POST, handleCaptureStop);
    server.on("/api/codes", HTTP_GET, handleCodes);
    server.on("/api/codes/raw", HTTP_GET, handleCodesRaw);
    server.on("/api/test", HTTP_POST, handleTest);
    server.on("/api/export", HTTP_GET, handleExport);
    server.on("/api/codes/clear", HTTP_POST, handleCodesClear);

    // Static files via 404 handler
    server.onNotFound(handleNotFound);

    server.begin();
    Serial.println("Web server started on port 80");
}

void handleIRCapture() {
    if (!isCapturing) return;

    if (irrecv.decode(&results)) {
        CapturedCode* targetCode = nullptr;
        const char* buttonDesc = "temp";

        if (isTempCapture) {
            // Temporary capture for dynamic button names
            targetCode = &tempCapture;
            buttonDesc = "dynamic temp button";
        } else if (captureButtonIndex >= 0 && captureButtonIndex < NUM_BUTTONS) {
            // Standard capture for predefined buttons
            targetCode = &capturedCodes[captureButtonIndex];
            buttonDesc = buttonNames[captureButtonIndex];
        }

        if (targetCode != nullptr) {
            // Store protocol info
            strncpy(targetCode->protocol, typeToString(results.decode_type).c_str(), sizeof(targetCode->protocol) - 1);
            targetCode->protocol[sizeof(targetCode->protocol) - 1] = '\0';
            targetCode->value = results.value;
            targetCode->bits = results.bits;

            // Store raw timing data
            targetCode->rawLen = min((uint16_t)(results.rawlen - 1), (uint16_t)MAX_RAW_LEN);
            for (uint16_t i = 0; i < targetCode->rawLen; i++) {
                targetCode->rawData[i] = results.rawbuf[i + 1] * kRawTick;
            }

            targetCode->captured = true;
            newCodeReceived = true;

            Serial.printf("Captured code for %s: protocol=%s, value=0x%llX, bits=%d, rawLen=%d\n",
                         buttonDesc,
                         targetCode->protocol,
                         targetCode->value,
                         targetCode->bits,
                         targetCode->rawLen);

            // Stop capturing after successful capture
            isCapturing = false;

            // Disable IR receiver to prevent conflicts with web server
            // Add delay to let pending interrupts complete
            delay(100);
            irrecv.disableIRIn();
            delay(50);
        } else {
            irrecv.resume();
        }
    }
}

int getButtonIndex(const String& name) {
    for (int i = 0; i < NUM_BUTTONS; i++) {
        if (name == buttonNames[i]) {
            return i;
        }
    }
    return -1;
}

String generateConfigH() {
    String output = "";
    output += "/*\n";
    output += " * IR Codes for SimpliFire Fireplace\n";
    output += " * Generated by IR Learner Tool\n";
    output += " */\n\n";
    output += "#ifndef IR_CODES_H\n";
    output += "#define IR_CODES_H\n\n";
    output += "#define IR_SEND_FREQ 36  // 36kHz for SimpliFire\n\n";

    // Map button names to config names
    const char* configNames[] = {
        "IR_RAW_POWER_ON",
        "IR_RAW_POWER_OFF",
        "IR_RAW_LIGHT_FROM_4",
        "IR_RAW_LIGHT_FROM_3",
        "IR_RAW_LIGHT_FROM_2",
        "IR_RAW_LIGHT_FROM_1",
        "IR_RAW_LIGHT_FROM_OFF",
        "IR_RAW_HEAT_ON",
        "IR_RAW_HEAT_OFF",
        "IR_RAW_HEAT_UP",
        "IR_RAW_HEAT_DOWN",
        "IR_RAW_TIMER_FROM_OFF",
        "IR_RAW_TIMER_FROM_0_5",
        "IR_RAW_TIMER_FROM_1",
        "IR_RAW_TIMER_FROM_2",
        "IR_RAW_TIMER_FROM_3",
        "IR_RAW_TIMER_FROM_4",
        "IR_RAW_TIMER_FROM_5",
        "IR_RAW_TIMER_FROM_6",
        "IR_RAW_TIMER_FROM_7",
        "IR_RAW_TIMER_FROM_8",
        "IR_RAW_TIMER_FROM_9"
    };

    for (int i = 0; i < NUM_BUTTONS; i++) {
        if (capturedCodes[i].captured) {
            output += "// " + String(buttonNames[i]) + "\n";
            output += "const uint16_t " + String(configNames[i]) + "[] = {\n    ";

            for (int j = 0; j < capturedCodes[i].rawLen; j++) {
                output += String(capturedCodes[i].rawData[j]);
                if (j < capturedCodes[i].rawLen - 1) {
                    output += ", ";
                }
                if ((j + 1) % 10 == 0 && j < capturedCodes[i].rawLen - 1) {
                    output += "\n    ";
                }
            }

            output += "\n};\n";
            output += "const uint16_t " + String(configNames[i]) + "_LEN = " + String(capturedCodes[i].rawLen) + ";\n\n";
        }
    }

    output += "#endif // IR_CODES_H\n";

    return output;
}
