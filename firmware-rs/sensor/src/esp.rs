use core::convert::TryInto;
use std::{
    net::Ipv4Addr,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context};
use dht_sensor::dht11;
use ds18b20::{Ds18b20, Resolution};
use embedded_svc::{
    http::{client::Client as HttpClient, Headers, Method, Status},
    io::{Read, Write},
    mqtt::client::QoS,
    wifi::{AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration},
};
use esp_idf_hal::{
    delay::Ets,
    gpio::{AnyIOPin, IOPin, InputOutput, PinDriver, Pull},
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{modem::Modem, prelude::Peripherals},
    http::{
        client::{Configuration as HttpClientConfiguration, EspHttpConnection},
        server::{Configuration as HttpConfiguration, EspHttpServer},
    },
    ipv4::{
        ClientConfiguration as IpClientConfiguration, ClientSettings as IpClientSettings,
        Configuration as IpConfiguration, Mask, Subnet,
    },
    log::EspLogger,
    mqtt::client::{EspMqttClient, MqttClientConfiguration},
    netif::{EspNetif, NetifConfiguration},
    nvs::{EspDefaultNvsPartition, EspNvs},
    ota::EspOta,
    wifi::{BlockingWifi, EspWifi},
};
use log::{info, warn};
use one_wire_bus::{Address, OneWire};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use thermostat_common::{
    config::NetworkConfig, RuntimeConfig, TOPIC_SENSOR_HUMIDITY, TOPIC_SENSOR_STATUS,
    TOPIC_SENSOR_TEMP,
};

const NVS_NAMESPACE: &str = "thermostat";
const NVS_RUNTIME_KEY: &str = "runtime_json";

const DS18B20_PIN: i32 = 4;
const DHT11_PIN: i32 = 16;

const PROVISIONING_AP_SSID: &str = "ThermostatSensor-AP";
const PROVISIONING_AP_PASSWORD: &str = "ThermostatSetup";
const MAX_HTTP_BODY: usize = 4096;
const OTA_CHUNK_SIZE: usize = 4096;
const WATCHDOG_TIMEOUT_SEC: u32 = 90;
const WIFI_RESTART_GRACE_MS: u64 = 300_000;
const WIFI_CONNECT_ATTEMPTS: u32 = 5;
const WIFI_RETRY_DELAY_MS: u64 = 3_000;

const SENSOR_PORTAL_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Thermostat Sensor Setup</title>
  <style>
    body{font-family:Arial,sans-serif;max-width:760px;margin:2rem auto;padding:0 1rem;color:#111}
    h1{margin:0 0 .5rem}.card{border:1px solid #ddd;border-radius:10px;padding:1rem;margin-bottom:1rem}
    label{display:block;margin:.5rem 0 .2rem}
    input[type=text],input[type=password],input[type=number]{width:100%;padding:.5rem;box-sizing:border-box}
    .row{display:flex;gap:1rem}.row>div{flex:1}
    .muted{color:#555}.ok{color:#106010}.err{color:#a00000}
    button{padding:.55rem .9rem;margin-top:.8rem}
    p{margin:.35rem 0}
  </style>
</head>
<body>
  <h1>Thermostat Sensor Setup</h1>
  <p class="muted">Configure WiFi/MQTT, then optionally apply an OTA image.</p>
  <p class="muted">Provisioning AP password: <code>ThermostatSetup</code></p>

  <div class="card">
    <h2>Network</h2>
    <label>WiFi SSID</label><input id="wifiSsid" type="text">
    <label>WiFi Password (leave blank to keep current)</label><input id="wifiPass" type="password">
    <div class="row">
      <div><label>MQTT Host</label><input id="mqttHost" type="text"></div>
      <div><label>MQTT Port</label><input id="mqttPort" type="number" min="1" max="65535"></div>
    </div>
    <label>MQTT Username</label><input id="mqttUser" type="text">
    <label>MQTT Password (leave blank to keep current)</label><input id="mqttPass" type="password">
    <label><input id="useStaticIp" type="checkbox"> Use static IP</label>
    <div class="row">
      <div><label>Static IP</label><input id="staticIp" type="text" placeholder="192.168.1.51"></div>
      <div><label>Gateway</label><input id="gateway" type="text" placeholder="192.168.1.1"></div>
    </div>
    <div class="row">
      <div><label>Subnet Mask</label><input id="subnet" type="text" placeholder="255.255.255.0"></div>
      <div><label>DNS</label><input id="dns" type="text" placeholder="192.168.1.1"></div>
    </div>
    <button id="save">Save Configuration</button>
    <button id="restart">Restart Device</button>
  </div>

  <div class="card">
    <h2>OTA</h2>
    <label>Firmware URL</label><input id="otaUrl" type="text" placeholder="https://example.com/sensor.bin">
    <label>SHA256 (optional)</label><input id="otaSha" type="text" placeholder="64 hex chars">
    <label>OTA Password (optional)</label><input id="otaPassword" type="password">
    <label><input id="otaReboot" type="checkbox" checked> Reboot after apply</label>
    <button id="otaApply">Apply OTA</button>
    <button id="otaRefresh">Refresh OTA Status</button>
    <p>Supported: <span id="otaSupported">--</span></p>
    <p>In Progress: <span id="otaInProgress">--</span></p>
    <p>Bytes: <span id="otaBytes">--</span></p>
    <p>Progress: <span id="otaProgress">--</span></p>
    <p>Last Error: <span id="otaLastError">--</span></p>
  </div>

  <p id="status" class="muted"></p>

  <script>
    const q=(id)=>document.getElementById(id);
    const toStr=(arr)=>Array.isArray(arr)?arr.join('.'):'';
    const toArr=(value)=>{if(!value.trim())return null;const p=value.trim().split('.').map(Number);if(p.length!==4||p.some(n=>!Number.isInteger(n)||n<0||n>255))throw new Error('Invalid IPv4: '+value);return p;};

    async function api(path,opt){
      const r=await fetch(path,opt);let b={};
      try{b=await r.json();}catch(_){}
      if(!r.ok)throw new Error(b.error||('Request failed: '+r.status));
      return b;
    }

    async function loadNetwork(){
      const n=await api('/api/network');
      q('wifiSsid').value=n.wifiSsid||'';
      q('mqttHost').value=n.mqttHost||'';
      q('mqttPort').value=n.mqttPort||1883;
      q('mqttUser').value=n.mqttUser||'';
      q('useStaticIp').checked=!!n.useStaticIp;
      q('staticIp').value=toStr(n.staticIp);
      q('gateway').value=toStr(n.gateway);
      q('subnet').value=toStr(n.subnet);
      q('dns').value=toStr(n.dns);
    }

    async function loadOta(){
      const s=await api('/api/ota/status');
      q('otaSupported').textContent=String(!!s.supported);
      q('otaInProgress').textContent=String(!!s.inProgress);
      q('otaBytes').textContent=s.totalBytes==null?String(s.bytesWritten||0):String((s.bytesWritten||0)+' / '+s.totalBytes);
      q('otaProgress').textContent=s.progressPct==null?'--':String(s.progressPct)+'%';
      q('otaLastError').textContent=s.lastError||'--';
    }

    q('save').addEventListener('click', async ()=>{
      q('status').className='muted'; q('status').textContent='Saving...';
      try{
        const payload={
          wifiSsid:q('wifiSsid').value.trim(),
          wifiPass:q('wifiPass').value||undefined,
          mqttHost:q('mqttHost').value.trim(),
          mqttPort:Number(q('mqttPort').value||1883),
          mqttUser:q('mqttUser').value.trim(),
          mqttPass:q('mqttPass').value||undefined,
          useStaticIp:q('useStaticIp').checked,
          staticIp:toArr(q('staticIp').value),
          gateway:toArr(q('gateway').value),
          subnet:toArr(q('subnet').value),
          dns:toArr(q('dns').value),
        };
        const res=await api('/api/network',{method:'PUT',headers:{'content-type':'application/json'},body:JSON.stringify(payload)});
        q('status').className='ok'; q('status').textContent='Saved. restartRequired='+String(!!res.restartRequired);
        q('wifiPass').value=''; q('mqttPass').value='';
      }catch(err){q('status').className='err'; q('status').textContent=err.message;}
    });

    q('restart').addEventListener('click', async ()=>{
      q('status').className='muted'; q('status').textContent='Restarting...';
      try{await api('/api/restart',{method:'POST'});q('status').className='ok';q('status').textContent='Restart requested.';}
      catch(err){q('status').className='err';q('status').textContent=err.message;}
    });

    q('otaApply').addEventListener('click', async ()=>{
      q('status').className='muted'; q('status').textContent='Starting OTA...';
      try{
        const url=q('otaUrl').value.trim();
        if(!url) throw new Error('Firmware URL is required');
        const payload={url,reboot:q('otaReboot').checked};
        const sha=q('otaSha').value.trim();
        if(sha) payload.sha256=sha;
        const password=q('otaPassword').value.trim();
        if(password) payload.password=password;
        await api('/api/ota/apply',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(payload)});
        q('status').className='ok'; q('status').textContent='OTA apply started.';
        await loadOta();
      }catch(err){q('status').className='err'; q('status').textContent=err.message;}
    });

    q('otaRefresh').addEventListener('click', ()=>loadOta().catch(err=>{q('status').className='err';q('status').textContent=err.message;}));

    Promise.all([loadNetwork(),loadOta()])
      .catch((err)=>{q('status').className='err';q('status').textContent=err.message;});
  </script>
</body>
</html>
"#;

enum WifiStartup {
    Connected(EspWifi<'static>),
    Provisioning(EspWifi<'static>),
}

struct SensorReadings {
    temperature_f: Option<f32>,
    humidity: Option<f32>,
}

struct SensorSuite {
    one_wire: OneWire<PinDriver<'static, AnyIOPin, InputOutput>>,
    ds18_address: Option<Address>,
    dht_pin: PinDriver<'static, AnyIOPin, InputOutput>,
    delay: Ets,
}

#[derive(Clone)]
struct NvsStore {
    partition: EspDefaultNvsPartition,
    lock: Arc<Mutex<()>>,
}

#[derive(Debug, Serialize)]
struct NetworkConfigView {
    #[serde(rename = "wifiSsid")]
    wifi_ssid: String,
    #[serde(rename = "wifiPassSet")]
    wifi_pass_set: bool,
    #[serde(rename = "mqttHost")]
    mqtt_host: String,
    #[serde(rename = "mqttPort")]
    mqtt_port: u16,
    #[serde(rename = "mqttUser")]
    mqtt_user: String,
    #[serde(rename = "mqttPassSet")]
    mqtt_pass_set: bool,
    #[serde(rename = "otaPasswordSet")]
    ota_password_set: bool,
    #[serde(rename = "useStaticIp")]
    use_static_ip: bool,
    #[serde(rename = "staticIp")]
    static_ip: Option<[u8; 4]>,
    gateway: Option<[u8; 4]>,
    subnet: Option<[u8; 4]>,
    dns: Option<[u8; 4]>,
}

#[derive(Debug, Deserialize)]
struct NetworkConfigUpdate {
    #[serde(rename = "wifiSsid")]
    wifi_ssid: String,
    #[serde(rename = "wifiPass", default)]
    wifi_pass: Option<String>,
    #[serde(rename = "mqttHost")]
    mqtt_host: String,
    #[serde(rename = "mqttPort")]
    mqtt_port: u16,
    #[serde(rename = "mqttUser")]
    mqtt_user: String,
    #[serde(rename = "mqttPass", default)]
    mqtt_pass: Option<String>,
    #[serde(rename = "otaPassword", default)]
    ota_password: Option<String>,
    #[serde(rename = "useStaticIp")]
    use_static_ip: bool,
    #[serde(rename = "staticIp")]
    static_ip: Option<[u8; 4]>,
    gateway: Option<[u8; 4]>,
    subnet: Option<[u8; 4]>,
    dns: Option<[u8; 4]>,
}

#[derive(Debug, Serialize)]
struct NetworkUpdateResponse {
    #[serde(rename = "restartRequired")]
    restart_required: bool,
    network: NetworkConfigView,
}

#[derive(Debug, Default)]
struct OtaRuntimeState {
    in_progress: bool,
    bytes_written: u64,
    total_bytes: Option<u64>,
    progress_pct: Option<u8>,
    last_error: Option<String>,
    last_sha256: Option<String>,
    last_source_url: Option<String>,
    last_completed_epoch: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct OtaApplyRequest {
    url: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    reboot: Option<bool>,
}

#[derive(Debug, Serialize)]
struct OtaApplyResponse {
    accepted: bool,
    #[serde(rename = "inProgress")]
    in_progress: bool,
}

#[derive(Debug, Serialize)]
struct OtaStatusResponse {
    supported: bool,
    #[serde(rename = "inProgress")]
    in_progress: bool,
    #[serde(rename = "bytesWritten")]
    bytes_written: u64,
    #[serde(rename = "totalBytes")]
    total_bytes: Option<u64>,
    #[serde(rename = "progressPct")]
    progress_pct: Option<u8>,
    #[serde(rename = "lastError")]
    last_error: Option<String>,
    #[serde(rename = "lastSha256")]
    last_sha256: Option<String>,
    #[serde(rename = "lastSourceUrl")]
    last_source_url: Option<String>,
    #[serde(rename = "lastCompletedEpoch")]
    last_completed_epoch: Option<i64>,
    #[serde(rename = "runningSlot")]
    running_slot: Option<String>,
    #[serde(rename = "bootSlot")]
    boot_slot: Option<String>,
    #[serde(rename = "updateSlot")]
    update_slot: Option<String>,
}

impl SensorSuite {
    fn new(ds18_pin: AnyIOPin, dht_pin: AnyIOPin) -> anyhow::Result<Self> {
        let mut one_wire_pin = PinDriver::input_output_od(ds18_pin)?;
        one_wire_pin.set_pull(Pull::Up)?;
        one_wire_pin.set_high()?;

        let mut dht_pin = PinDriver::input_output_od(dht_pin)?;
        dht_pin.set_pull(Pull::Up)?;
        dht_pin.set_high()?;

        let one_wire = OneWire::new(one_wire_pin)
            .map_err(|err| anyhow!("failed to initialize one-wire bus: {err:?}"))?;

        let mut suite = Self {
            one_wire,
            ds18_address: None,
            dht_pin,
            delay: Ets,
        };

        suite.refresh_ds18_address();
        Ok(suite)
    }

    fn read(&mut self) -> SensorReadings {
        SensorReadings {
            temperature_f: self.read_temperature_f(),
            humidity: self.read_humidity(),
        }
    }

    fn refresh_ds18_address(&mut self) {
        let mut first_ds18: Option<Address> = None;
        let mut device_count = 0_u32;

        for addr in self.one_wire.devices(false, &mut self.delay) {
            match addr {
                Ok(address) => {
                    device_count = device_count.saturating_add(1);
                    if first_ds18.is_none() && address.family_code() == ds18b20::FAMILY_CODE {
                        first_ds18 = Some(address);
                    }
                }
                Err(err) => {
                    warn!("one-wire device scan failed: {err:?}");
                    break;
                }
            }
        }

        self.ds18_address = first_ds18;

        if let Some(address) = self.ds18_address {
            info!(
                "DS18B20 ready on GPIO{} ({} one-wire device(s), using {:?})",
                DS18B20_PIN, device_count, address
            );
        } else {
            warn!(
                "no DS18B20 found on GPIO{} ({} one-wire device(s) detected)",
                DS18B20_PIN, device_count
            );
        }
    }

    fn read_temperature_f(&mut self) -> Option<f32> {
        if self.ds18_address.is_none() {
            self.refresh_ds18_address();
        }

        let address = self.ds18_address?;
        let sensor = match Ds18b20::new::<core::convert::Infallible>(address) {
            Ok(sensor) => sensor,
            Err(err) => {
                warn!("invalid DS18B20 address {:?}: {err:?}", address);
                self.ds18_address = None;
                return None;
            }
        };

        if let Err(err) =
            ds18b20::start_simultaneous_temp_measurement(&mut self.one_wire, &mut self.delay)
        {
            warn!("failed to start DS18B20 conversion: {err:?}");
            self.ds18_address = None;
            return None;
        }

        Resolution::Bits12.delay_for_measurement_time(&mut self.delay);

        match sensor.read_data(&mut self.one_wire, &mut self.delay) {
            Ok(data) => {
                let temp_f = celsius_to_fahrenheit(data.temperature);
                info!(
                    "[DS18B20] Temperature: {:.1}°F ({:.1}°C)",
                    temp_f, data.temperature
                );
                Some(temp_f)
            }
            Err(err) => {
                warn!("failed to read DS18B20 data: {err:?}");
                self.ds18_address = None;
                None
            }
        }
    }

    fn read_humidity(&mut self) -> Option<f32> {
        if let Err(err) = self.dht_pin.set_high() {
            warn!("failed to set DHT11 line high before read: {err:?}");
            return None;
        }

        match dht11::blocking::read(&mut self.delay, &mut self.dht_pin) {
            Ok(reading) => {
                let humidity = reading.relative_humidity as f32;
                info!("[DHT11] Humidity: {:.1}%", humidity);
                Some(humidity)
            }
            Err(err) => {
                warn!(
                    "failed to read DHT11 humidity on GPIO{}: {err:?}",
                    DHT11_PIN
                );
                None
            }
        }
    }
}

pub fn run() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let sys_loop = EspSystemEventLoop::take()?;
    let nvs_partition = EspDefaultNvsPartition::take()?;
    let nvs_store = NvsStore {
        partition: nvs_partition.clone(),
        lock: Arc::new(Mutex::new(())),
    };

    let mut runtime = nvs_store.load_runtime_config().unwrap_or_else(|err| {
        warn!("failed to load runtime config from NVS: {err:#}");
        RuntimeConfig::default()
    });

    ensure_wifi_defaults(&mut runtime);

    let Peripherals { modem, pins, .. } = Peripherals::take()?;

    let mut sensors = SensorSuite::new(pins.gpio4.downgrade(), pins.gpio16.downgrade())
        .context("failed to initialize sensor suite")?;

    let wifi = match connect_wifi(modem, sys_loop.clone(), nvs_partition, &runtime.network)
        .context("wifi startup failed")?
    {
        WifiStartup::Connected(wifi) => {
            info!("wifi connected");
            wifi
        }
        WifiStartup::Provisioning(wifi) => {
            warn!(
                "wifi station connection unavailable; starting provisioning AP `{}`",
                PROVISIONING_AP_SSID
            );
            let server = create_provisioning_http_server(nvs_store.clone())?;

            let _wifi = wifi;
            let _server = server;
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }
    };
    disable_wifi_power_save();

    if let Ok(mut ota) = EspOta::new() {
        if let Err(err) = ota.mark_running_slot_valid() {
            warn!("failed to mark running OTA slot valid: {err:?}");
        }
    }

    init_watchdog(WATCHDOG_TIMEOUT_SEC)?;
    add_current_task_to_watchdog()?;

    let ota_state = Arc::new(Mutex::new(OtaRuntimeState::default()));
    let server = create_http_server(nvs_store.clone(), ota_state.clone())?;

    let (mut mqtt, mut conn) = create_mqtt_client(&runtime)?;

    thread::Builder::new()
        .name("mqtt-poll".to_string())
        .stack_size(8192)
        .spawn(move || {
            loop {
                match conn.next() {
                    Ok(_event) => {
                        // Sensor node currently has no command subscriptions.
                    }
                    Err(err) => {
                        warn!("sensor mqtt poll error: {err:?}");
                        thread::sleep(Duration::from_secs(2));
                    }
                }
            }
        })
        .expect("failed to spawn mqtt thread");

    if let Err(err) = mqtt.publish(TOPIC_SENSOR_STATUS, QoS::AtLeastOnce, true, b"online") {
        warn!("failed to publish sensor online status: {err:?}");
    }

    // Keep services alive for the program lifetime.
    let _wifi = wifi;
    let _server = server;
    let mut wifi_disconnected_since: Option<Instant> = None;

    loop {
        feed_watchdog();
        maintain_wifi_health(&mut wifi_disconnected_since);

        let readings = sensors.read();

        if let Some(temp_f) = readings.temperature_f {
            let temp_payload = format!("{temp_f:.1}");
            if let Err(err) = mqtt.publish(
                TOPIC_SENSOR_TEMP,
                QoS::AtLeastOnce,
                true,
                temp_payload.as_bytes(),
            ) {
                warn!("failed to publish temperature: {err:?}");
            }
        }

        if let Some(humidity) = readings.humidity {
            let humidity_payload = format!("{humidity:.1}");
            if let Err(err) = mqtt.publish(
                TOPIC_SENSOR_HUMIDITY,
                QoS::AtLeastOnce,
                true,
                humidity_payload.as_bytes(),
            ) {
                warn!("failed to publish humidity: {err:?}");
            }
        }

        for _ in 0..30 {
            feed_watchdog();
            maintain_wifi_health(&mut wifi_disconnected_since);
            thread::sleep(Duration::from_secs(1));
        }
    }
}

fn create_http_server(
    nvs_store: NvsStore,
    ota_state: Arc<Mutex<OtaRuntimeState>>,
) -> anyhow::Result<EspHttpServer<'static>> {
    let conf = HttpConfiguration {
        stack_size: 16 * 1024,
        ..Default::default()
    };

    let mut server = EspHttpServer::new(&conf)?;

    server.fn_handler::<anyhow::Error, _>("/", Method::Get, move |req| {
        req.into_response(200, Some("OK"), &[("Content-Type", "text/html; charset=utf-8")])?
            .write_all(SENSOR_PORTAL_HTML.as_bytes())?;
        Ok(())
    })?;

    {
        let nvs_store = nvs_store.clone();
        server.fn_handler("/api/network", Method::Get, move |req| {
            let runtime = nvs_store.load_runtime_config().unwrap_or_default();
            let payload = build_network_config_view(&runtime.network);
            write_json(req, &payload)
        })?;
    }

    {
        let nvs_store = nvs_store.clone();
        server.fn_handler::<anyhow::Error, _>("/api/network", Method::Put, move |mut req| {
            let body = read_request_body(&mut req)?;
            let update: NetworkConfigUpdate =
                serde_json::from_slice(&body).context("invalid network payload")?;

            if let Err(message) = validate_network_update(&update) {
                return write_error(req, 400, message);
            }

            let payload = apply_network_update(&nvs_store, update)?;
            write_json(req, &payload)
        })?;
    }

    {
        let ota_state = ota_state.clone();
        server.fn_handler("/api/ota/status", Method::Get, move |req| {
            let payload = build_ota_status_response(&ota_state);
            write_json(req, &payload)
        })?;
    }

    {
        let ota_state = ota_state.clone();
        let nvs_store = nvs_store.clone();
        server.fn_handler::<anyhow::Error, _>("/api/ota/apply", Method::Post, move |mut req| {
            let body = read_request_body(&mut req)?;
            let update: OtaApplyRequest =
                serde_json::from_slice(&body).context("invalid ota payload")?;

            if let Err(message) = validate_ota_apply_request(&update) {
                return write_error(req, 400, message);
            }

            match apply_ota_update(&ota_state, &nvs_store, update) {
                Ok(payload) => write_json(req, &payload),
                Err(err) => {
                    let message = err.to_string();
                    if message.contains("invalid OTA password") {
                        write_error(req, 403, &message)
                    } else if message.contains("already in progress") {
                        write_error(req, 409, &message)
                    } else {
                        write_error(req, 500, "Failed to start OTA apply")
                    }
                }
            }
        })?;
    }

    server.fn_handler("/api/restart", Method::Post, move |req| {
        thread::Builder::new()
            .name("restart-request".into())
            .spawn(|| {
                thread::sleep(Duration::from_millis(500));
                unsafe { esp_idf_svc::sys::esp_restart() };
            })
            .expect("failed to spawn restart thread");

        let payload = serde_json::json!({ "restarting": true });
        write_json(req, &payload)
    })?;

    Ok(server)
}

fn create_provisioning_http_server(nvs_store: NvsStore) -> anyhow::Result<EspHttpServer<'static>> {
    let conf = HttpConfiguration {
        stack_size: 16 * 1024,
        ..Default::default()
    };

    let mut server = EspHttpServer::new(&conf)?;

    for path in [
        "/",
        "/generate_204",
        "/gen_204",
        "/hotspot-detect.html",
        "/connecttest.txt",
        "/ncsi.txt",
        "/fwlink",
    ] {
        server.fn_handler::<anyhow::Error, _>(path, Method::Get, move |req| {
            req.into_response(200, Some("OK"), &[("Content-Type", "text/html; charset=utf-8")])?
                .write_all(SENSOR_PORTAL_HTML.as_bytes())?;
            Ok(())
        })?;
    }

    {
        let nvs_store = nvs_store.clone();
        server.fn_handler("/api/network", Method::Get, move |req| {
            let runtime = nvs_store.load_runtime_config().unwrap_or_default();
            let payload = build_network_config_view(&runtime.network);
            write_json(req, &payload)
        })?;
    }

    {
        let nvs_store = nvs_store.clone();
        server.fn_handler::<anyhow::Error, _>("/api/network", Method::Put, move |mut req| {
            let body = read_request_body(&mut req)?;
            let update: NetworkConfigUpdate =
                serde_json::from_slice(&body).context("invalid network payload")?;

            if let Err(message) = validate_network_update(&update) {
                return write_error(req, 400, message);
            }

            let payload = apply_network_update(&nvs_store, update)?;
            write_json(req, &payload)
        })?;
    }

    server.fn_handler("/api/ota/status", Method::Get, move |req| {
        let payload = OtaStatusResponse {
            supported: false,
            in_progress: false,
            bytes_written: 0,
            total_bytes: None,
            progress_pct: None,
            last_error: Some(
                "Sensor is in provisioning mode; OTA apply is unavailable".to_string(),
            ),
            last_sha256: None,
            last_source_url: None,
            last_completed_epoch: None,
            running_slot: None,
            boot_slot: None,
            update_slot: None,
        };
        write_json(req, &payload)
    })?;

    server.fn_handler("/api/ota/apply", Method::Post, move |req| {
        write_error(req, 409, "Connect station WiFi before applying OTA updates")
    })?;

    server.fn_handler("/api/restart", Method::Post, move |req| {
        thread::Builder::new()
            .name("restart-request".into())
            .spawn(|| {
                thread::sleep(Duration::from_millis(500));
                unsafe { esp_idf_svc::sys::esp_restart() };
            })
            .expect("failed to spawn restart thread");

        let payload = serde_json::json!({ "restarting": true });
        write_json(req, &payload)
    })?;

    Ok(server)
}

fn read_request_body(
    req: &mut esp_idf_svc::http::server::Request<
        &mut esp_idf_svc::http::server::EspHttpConnection<'_>,
    >,
) -> anyhow::Result<Vec<u8>> {
    let len = req.content_len().unwrap_or(0) as usize;
    if len > MAX_HTTP_BODY {
        return Err(anyhow!("request body too large"));
    }

    let mut body = vec![0_u8; len];
    if len > 0 {
        req.read_exact(&mut body)?;
    }
    Ok(body)
}

fn write_json<T: Serialize>(
    mut req: esp_idf_svc::http::server::Request<
        &mut esp_idf_svc::http::server::EspHttpConnection<'_>,
    >,
    payload: &T,
) -> anyhow::Result<()> {
    let body = serde_json::to_vec(payload)?;
    req.into_response(
        200,
        Some("OK"),
        &[("Content-Type", "application/json; charset=utf-8")],
    )?
    .write_all(&body)?;
    Ok(())
}

fn write_error(
    mut req: esp_idf_svc::http::server::Request<
        &mut esp_idf_svc::http::server::EspHttpConnection<'_>,
    >,
    status_code: u16,
    message: &str,
) -> anyhow::Result<()> {
    let payload = serde_json::json!({ "error": message });
    let body = serde_json::to_vec(&payload)?;
    req.into_response(
        status_code,
        None,
        &[("Content-Type", "application/json; charset=utf-8")],
    )?
    .write_all(&body)?;
    Ok(())
}

fn has_station_credentials(network: &NetworkConfig) -> bool {
    let ssid = network.wifi_ssid.trim();
    !ssid.is_empty() && ssid != "CHANGE_ME"
}

fn ensure_wifi_defaults(runtime: &mut RuntimeConfig) {
    if runtime.network.wifi_ssid.is_empty() {
        runtime.network.wifi_ssid = option_env!("WIFI_SSID").unwrap_or("CHANGE_ME").to_string();
    }

    if runtime.network.wifi_pass.is_empty() {
        runtime.network.wifi_pass = option_env!("WIFI_PASS").unwrap_or("CHANGE_ME").to_string();
    }
}

fn ipv4_from_octets(ip: [u8; 4]) -> Ipv4Addr {
    Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3])
}

fn build_static_ip_config(network: &NetworkConfig) -> anyhow::Result<Option<NetifConfiguration>> {
    if !network.use_static_ip {
        return Ok(None);
    }

    let static_ip = network
        .static_ip
        .ok_or_else(|| anyhow!("staticIp is required when useStaticIp is true"))?;
    let gateway = network
        .gateway
        .ok_or_else(|| anyhow!("gateway is required when useStaticIp is true"))?;
    let subnet = network
        .subnet
        .ok_or_else(|| anyhow!("subnet is required when useStaticIp is true"))?;

    let mask_ip = ipv4_from_octets(subnet);
    let mask = Mask::try_from(mask_ip).map_err(|_| anyhow!("invalid subnet mask: {}", mask_ip))?;

    let mut conf = NetifConfiguration::wifi_default_client();
    conf.key = "WIFI_STA_STATIC".try_into().unwrap();
    conf.ip_configuration = Some(IpConfiguration::Client(IpClientConfiguration::Fixed(
        IpClientSettings {
            ip: ipv4_from_octets(static_ip),
            subnet: Subnet {
                gateway: ipv4_from_octets(gateway),
                mask,
            },
            dns: network.dns.map(ipv4_from_octets),
            secondary_dns: None,
        },
    )));

    Ok(Some(conf))
}

fn connect_wifi(
    modem: Modem,
    sys_loop: EspSystemEventLoop,
    nvs_partition: EspDefaultNvsPartition,
    network: &NetworkConfig,
) -> anyhow::Result<WifiStartup> {
    let mut esp_wifi = EspWifi::new(modem, sys_loop.clone(), Some(nvs_partition))?;

    let static_ip_error = match build_static_ip_config(network) {
        Ok(Some(conf)) => match EspNetif::new_with_conf(&conf) {
            Ok(sta_netif) => {
                esp_wifi
                    .swap_netif_sta(sta_netif)
                    .context("failed to apply static IP netif configuration")?;
                None
            }
            Err(err) => Some(anyhow::Error::from(err).context("failed to create static IP netif")),
        },
        Ok(None) => None,
        Err(err) => Some(err),
    };

    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sys_loop)?;

    if let Some(err) = static_ip_error {
        warn!("invalid static IP configuration ({err:#}); entering provisioning mode");
        start_provisioning_ap(&mut wifi)?;
        return Ok(WifiStartup::Provisioning(esp_wifi));
    }

    if !has_station_credentials(network) {
        warn!("wifi credentials missing; entering provisioning AP mode");
        start_provisioning_ap(&mut wifi)?;
        return Ok(WifiStartup::Provisioning(esp_wifi));
    }

    let auth_method = if network.wifi_pass.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPAWPA2Personal
    };

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: network
            .wifi_ssid
            .as_str()
            .try_into()
            .map_err(|_| anyhow!("wifi ssid too long"))?,
        password: network
            .wifi_pass
            .as_str()
            .try_into()
            .map_err(|_| anyhow!("wifi password too long"))?,
        auth_method,
        ..Default::default()
    }))?;

    wifi.start()?;
    info!("wifi started, connecting to `{}`", network.wifi_ssid);

    let mut last_err = None;
    for attempt in 1..=WIFI_CONNECT_ATTEMPTS {
        info!("wifi connect attempt {attempt}/{WIFI_CONNECT_ATTEMPTS}");
        match wifi.connect() {
            Ok(()) => match wifi.wait_netif_up() {
                Ok(()) => {
                    info!("wifi connected and netif up on attempt {attempt}");
                    last_err = None;
                    break;
                }
                Err(err) => {
                    warn!("wifi netif up failed on attempt {attempt}: {err:#}");
                    last_err = Some(err);
                }
            },
            Err(err) => {
                warn!("wifi connect failed on attempt {attempt}: {err:#}");
                last_err = Some(err);
            }
        }

        if attempt < WIFI_CONNECT_ATTEMPTS {
            let _ = wifi.disconnect();
            thread::sleep(Duration::from_millis(WIFI_RETRY_DELAY_MS));
        }
    }

    match last_err {
        None => Ok(WifiStartup::Connected(esp_wifi)),
        Some(err) => {
            warn!("all {WIFI_CONNECT_ATTEMPTS} wifi connect attempts failed; last error: {err:#}");
            let _ = wifi.disconnect();
            let _ = wifi.stop();
            start_provisioning_ap(&mut wifi)?;
            Ok(WifiStartup::Provisioning(esp_wifi))
        }
    }
}

fn start_provisioning_ap(wifi: &mut BlockingWifi<&mut EspWifi<'static>>) -> anyhow::Result<()> {
    wifi.set_configuration(&Configuration::AccessPoint(AccessPointConfiguration {
        ssid: PROVISIONING_AP_SSID
            .try_into()
            .map_err(|_| anyhow!("provisioning AP SSID too long"))?,
        password: PROVISIONING_AP_PASSWORD
            .try_into()
            .map_err(|_| anyhow!("provisioning AP password too long"))?,
        auth_method: AuthMethod::WPAWPA2Personal,
        channel: 1,
        ..Default::default()
    }))?;
    wifi.start()?;
    wifi.wait_netif_up()?;
    info!(
        "provisioning AP started on `{}` (password: `{}`)",
        PROVISIONING_AP_SSID, PROVISIONING_AP_PASSWORD
    );
    Ok(())
}

fn validate_network_update(update: &NetworkConfigUpdate) -> Result<(), &'static str> {
    if update.wifi_ssid.trim().is_empty() {
        return Err("wifiSsid cannot be empty");
    }
    if update.mqtt_host.trim().is_empty() {
        return Err("mqttHost cannot be empty");
    }
    if update.mqtt_port == 0 {
        return Err("mqttPort must be between 1 and 65535");
    }
    if update.use_static_ip
        && (update.static_ip.is_none() || update.gateway.is_none() || update.subnet.is_none())
    {
        return Err("staticIp, gateway, and subnet are required when useStaticIp is true");
    }

    Ok(())
}

fn apply_network_update(
    nvs_store: &NvsStore,
    update: NetworkConfigUpdate,
) -> anyhow::Result<NetworkUpdateResponse> {
    let mut runtime = nvs_store.load_runtime_config().unwrap_or_default();
    let previous = runtime.network.clone();

    runtime.network.wifi_ssid = update.wifi_ssid;
    if let Some(pass) = update.wifi_pass {
        runtime.network.wifi_pass = pass;
    }
    runtime.network.mqtt_host = update.mqtt_host;
    runtime.network.mqtt_port = update.mqtt_port;
    runtime.network.mqtt_user = update.mqtt_user;
    if let Some(pass) = update.mqtt_pass {
        runtime.network.mqtt_pass = pass;
    }
    if let Some(pass) = update.ota_password {
        runtime.network.ota_password = pass;
    }
    runtime.network.use_static_ip = update.use_static_ip;
    runtime.network.static_ip = update.static_ip;
    runtime.network.gateway = update.gateway;
    runtime.network.subnet = update.subnet;
    runtime.network.dns = update.dns;

    nvs_store.save_runtime_config(&runtime)?;

    Ok(NetworkUpdateResponse {
        restart_required: network_restart_required(&previous, &runtime.network),
        network: build_network_config_view(&runtime.network),
    })
}

fn build_network_config_view(network: &NetworkConfig) -> NetworkConfigView {
    NetworkConfigView {
        wifi_ssid: network.wifi_ssid.clone(),
        wifi_pass_set: !network.wifi_pass.is_empty(),
        mqtt_host: network.mqtt_host.clone(),
        mqtt_port: network.mqtt_port,
        mqtt_user: network.mqtt_user.clone(),
        mqtt_pass_set: !network.mqtt_pass.is_empty(),
        ota_password_set: !network.ota_password.is_empty(),
        use_static_ip: network.use_static_ip,
        static_ip: network.static_ip,
        gateway: network.gateway,
        subnet: network.subnet,
        dns: network.dns,
    }
}

fn network_restart_required(previous: &NetworkConfig, current: &NetworkConfig) -> bool {
    previous.wifi_ssid != current.wifi_ssid
        || previous.wifi_pass != current.wifi_pass
        || previous.use_static_ip != current.use_static_ip
        || previous.static_ip != current.static_ip
        || previous.gateway != current.gateway
        || previous.subnet != current.subnet
        || previous.dns != current.dns
        || previous.mqtt_host != current.mqtt_host
        || previous.mqtt_port != current.mqtt_port
        || previous.mqtt_user != current.mqtt_user
        || previous.mqtt_pass != current.mqtt_pass
}

fn validate_ota_apply_request(update: &OtaApplyRequest) -> Result<(), &'static str> {
    let url = update.url.trim();
    if url.is_empty() {
        return Err("url cannot be empty");
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("url must start with http:// or https://");
    }

    if let Some(sha256) = update.sha256.as_ref() {
        let value = sha256.trim();
        if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("sha256 must be 64 hex characters");
        }
    }

    Ok(())
}

fn apply_ota_update(
    ota_state: &Arc<Mutex<OtaRuntimeState>>,
    nvs_store: &NvsStore,
    update: OtaApplyRequest,
) -> anyhow::Result<OtaApplyResponse> {
    let runtime = nvs_store.load_runtime_config().unwrap_or_default();
    if !runtime.network.ota_password.is_empty() {
        let supplied = update.password.as_deref().unwrap_or_default();
        if supplied != runtime.network.ota_password {
            return Err(anyhow!("invalid OTA password"));
        }
    }

    {
        let mut ota = ota_state.lock().unwrap();
        if ota.in_progress {
            return Err(anyhow!("OTA update already in progress"));
        }

        ota.in_progress = true;
        ota.bytes_written = 0;
        ota.total_bytes = None;
        ota.progress_pct = None;
        ota.last_error = None;
        ota.last_sha256 = None;
        ota.last_source_url = Some(update.url.clone());
    }

    let ota_state_for_thread = ota_state.clone();
    let spawn_result = thread::Builder::new()
        .name("ota-apply".into())
        .stack_size(16 * 1024)
        .spawn(move || {
            let reboot_after_apply = update.reboot.unwrap_or(true);
            let expected_sha = update
                .sha256
                .as_ref()
                .map(|v| v.trim().to_ascii_lowercase());
            let result =
                download_and_apply_ota(&ota_state_for_thread, &update.url, expected_sha.as_deref());

            match result {
                Ok((bytes_written, digest_hex)) => {
                    {
                        let mut ota = ota_state_for_thread.lock().unwrap();
                        ota.in_progress = false;
                        ota.bytes_written = bytes_written;
                        ota.progress_pct = Some(100);
                        ota.last_error = None;
                        ota.last_sha256 = Some(digest_hex);
                        ota.last_completed_epoch = Some(chrono::Utc::now().timestamp());
                    }

                    info!(
                        "sensor OTA apply completed successfully ({} bytes)",
                        bytes_written
                    );

                    if reboot_after_apply {
                        thread::sleep(Duration::from_millis(800));
                        unsafe { esp_idf_svc::sys::esp_restart() };
                    }
                }
                Err(err) => {
                    warn!("sensor OTA apply failed: {err:#}");
                    let mut ota = ota_state_for_thread.lock().unwrap();
                    ota.in_progress = false;
                    ota.last_error = Some(err.to_string());
                    ota.last_completed_epoch = Some(chrono::Utc::now().timestamp());
                }
            }
        });

    if let Err(err) = spawn_result {
        let message = format!("failed to spawn OTA apply thread: {err}");
        let mut ota = ota_state.lock().unwrap();
        ota.in_progress = false;
        ota.last_error = Some(message.clone());
        return Err(anyhow!("{message}"));
    }

    Ok(OtaApplyResponse {
        accepted: true,
        in_progress: true,
    })
}

fn download_and_apply_ota(
    ota_state: &Arc<Mutex<OtaRuntimeState>>,
    url: &str,
    expected_sha256: Option<&str>,
) -> anyhow::Result<(u64, String)> {
    let http_conf = HttpClientConfiguration {
        timeout: Some(Duration::from_secs(30)),
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        ..Default::default()
    };
    let mut client = HttpClient::wrap(EspHttpConnection::new(&http_conf)?);
    let request = client.request(Method::Get, url, &[])?;
    let mut response = request.submit().map_err(|e| anyhow!("{e:?}"))?;

    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(anyhow!("OTA download failed with HTTP {status}"));
    }

    let content_length = response
        .header("content-length")
        .or_else(|| response.header("Content-Length"))
        .and_then(|value| value.parse::<u64>().ok());

    {
        let mut ota = ota_state.lock().unwrap();
        ota.total_bytes = content_length;
    }

    let mut ota = EspOta::new().map_err(|err| anyhow!("failed to acquire OTA: {err:?}"))?;
    let mut update = ota
        .initiate_update()
        .map_err(|err| anyhow!("failed to initiate OTA update: {err:?}"))?;

    let mut hasher = Sha256::new();
    let mut total_written = 0_u64;
    let mut chunk = [0_u8; OTA_CHUNK_SIZE];

    loop {
        let read = response.read(&mut chunk).map_err(|e| anyhow!("{e:?}"))?;
        if read == 0 {
            break;
        }

        update
            .write(&chunk[..read])
            .map_err(|err| anyhow!("failed writing OTA data: {err:?}"))?;
        hasher.update(&chunk[..read]);
        total_written = total_written.saturating_add(read as u64);

        let mut state = ota_state.lock().unwrap();
        state.bytes_written = total_written;
        if let Some(total) = state.total_bytes.filter(|value| *value > 0) {
            let pct = (total_written.saturating_mul(100) / total).min(100);
            state.progress_pct = Some(pct as u8);
        }
    }

    if total_written == 0 {
        return Err(anyhow!("OTA download body is empty"));
    }

    let digest = hasher.finalize();
    let mut digest_hex = String::with_capacity(64);
    for byte in digest {
        use core::fmt::Write as _;
        let _ = write!(&mut digest_hex, "{byte:02x}");
    }

    if let Some(expected) = expected_sha256 {
        let normalized = expected.trim().to_ascii_lowercase();
        if digest_hex != normalized {
            return Err(anyhow!(
                "sha256 mismatch (expected {normalized}, got {digest_hex})"
            ));
        }
    }

    update
        .complete()
        .map_err(|err| anyhow!("failed finalizing OTA image: {err:?}"))?;
    drop(ota);

    Ok((total_written, digest_hex))
}

enum SlotQuery {
    Running,
    Boot,
    Update,
}

fn ota_slot_label(query: SlotQuery) -> Option<String> {
    let ota = EspOta::new().ok()?;
    let slot = match query {
        SlotQuery::Running => ota.get_running_slot().ok()?,
        SlotQuery::Boot => ota.get_boot_slot().ok()?,
        SlotQuery::Update => ota.get_update_slot().ok()?,
    };
    Some(slot.label.as_str().to_string())
}

fn build_ota_status_response(ota_state: &Arc<Mutex<OtaRuntimeState>>) -> OtaStatusResponse {
    let ota = ota_state.lock().unwrap();

    OtaStatusResponse {
        supported: true,
        in_progress: ota.in_progress,
        bytes_written: ota.bytes_written,
        total_bytes: ota.total_bytes,
        progress_pct: ota.progress_pct,
        last_error: ota.last_error.clone(),
        last_sha256: ota.last_sha256.clone(),
        last_source_url: ota.last_source_url.clone(),
        last_completed_epoch: ota.last_completed_epoch,
        running_slot: ota_slot_label(SlotQuery::Running),
        boot_slot: ota_slot_label(SlotQuery::Boot),
        update_slot: ota_slot_label(SlotQuery::Update),
    }
}

fn create_mqtt_client(
    runtime: &RuntimeConfig,
) -> anyhow::Result<(
    EspMqttClient<'static>,
    esp_idf_svc::mqtt::client::EspMqttConnection,
)> {
    let url = format!(
        "mqtt://{}:{}",
        runtime.network.mqtt_host, runtime.network.mqtt_port
    );

    let conf = MqttClientConfiguration {
        client_id: Some("thermostat-sensor"),
        username: if runtime.network.mqtt_user.is_empty() {
            None
        } else {
            Some(runtime.network.mqtt_user.as_str())
        },
        password: if runtime.network.mqtt_pass.is_empty() {
            None
        } else {
            Some(runtime.network.mqtt_pass.as_str())
        },
        ..Default::default()
    };

    Ok(EspMqttClient::new(&url, &conf)?)
}

impl NvsStore {
    fn load_runtime_config(&self) -> anyhow::Result<RuntimeConfig> {
        let _guard = self.lock.lock().unwrap();
        let mut nvs = EspNvs::new(self.partition.clone(), NVS_NAMESPACE, true)?;
        let mut buffer = vec![0_u8; 4096];

        match nvs.get_str(NVS_RUNTIME_KEY, &mut buffer)? {
            Some(value) => Ok(serde_json::from_str::<RuntimeConfig>(value)?),
            None => Ok(RuntimeConfig::default()),
        }
    }

    fn save_runtime_config(&self, runtime: &RuntimeConfig) -> anyhow::Result<()> {
        let _guard = self.lock.lock().unwrap();
        let mut nvs = EspNvs::new(self.partition.clone(), NVS_NAMESPACE, true)?;
        let payload = serde_json::to_string(runtime)?;
        nvs.set_str(NVS_RUNTIME_KEY, &payload)?;
        Ok(())
    }
}

fn init_watchdog(timeout_sec: u32) -> anyhow::Result<()> {
    let config = esp_idf_svc::sys::esp_task_wdt_config_t {
        timeout_ms: timeout_sec.saturating_mul(1000),
        idle_core_mask: 0,
        trigger_panic: true,
    };
    let rc = unsafe { esp_idf_svc::sys::esp_task_wdt_init(&config) };
    if rc == esp_idf_svc::sys::ESP_OK || rc == esp_idf_svc::sys::ESP_ERR_INVALID_STATE {
        return Ok(());
    }
    Err(anyhow!("esp_task_wdt_init failed with code {}", rc))
}

fn add_current_task_to_watchdog() -> anyhow::Result<()> {
    let rc = unsafe { esp_idf_svc::sys::esp_task_wdt_add(core::ptr::null_mut()) };
    if rc == esp_idf_svc::sys::ESP_OK || rc == esp_idf_svc::sys::ESP_ERR_INVALID_STATE {
        return Ok(());
    }
    Err(anyhow!("esp_task_wdt_add failed with code {}", rc))
}

fn feed_watchdog() {
    let _ = unsafe { esp_idf_svc::sys::esp_task_wdt_reset() };
}

fn disable_wifi_power_save() {
    let rc = unsafe { esp_idf_svc::sys::esp_wifi_set_ps(0) };
    if rc == esp_idf_svc::sys::ESP_OK {
        info!("wifi power save disabled");
    } else {
        warn!("failed to disable wifi power save: esp_err_t={rc}");
    }
}

fn is_wifi_station_connected() -> bool {
    let mut ap_info = esp_idf_svc::sys::wifi_ap_record_t::default();
    let rc = unsafe { esp_idf_svc::sys::esp_wifi_sta_get_ap_info(&mut ap_info) };
    rc == esp_idf_svc::sys::ESP_OK
}

fn maintain_wifi_health(wifi_disconnected_since: &mut Option<Instant>) {
    if is_wifi_station_connected() {
        *wifi_disconnected_since = None;
        return;
    }

    match wifi_disconnected_since {
        Some(disconnected_since)
            if disconnected_since.elapsed().as_millis() as u64 >= WIFI_RESTART_GRACE_MS =>
        {
            warn!(
                "wifi disconnected for {}s; restarting device for recovery",
                WIFI_RESTART_GRACE_MS / 1000
            );
            thread::sleep(Duration::from_millis(100));
            unsafe { esp_idf_svc::sys::esp_restart() };
        }
        Some(_) => {}
        None => *wifi_disconnected_since = Some(Instant::now()),
    }
}

fn celsius_to_fahrenheit(temp_c: f32) -> f32 {
    temp_c * 9.0 / 5.0 + 32.0
}
