use core::convert::TryInto;
use std::{
    collections::HashMap,
    net::Ipv4Addr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context};
use chrono::{Offset, Utc};
use chrono_tz::Tz;
use embedded_svc::{
    http::{client::Client as HttpClient, Headers, Method, Status},
    io::{Read, Write},
    mqtt::client::{Details, EventPayload, QoS},
    wifi::{AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration},
};
use esp_idf_hal::gpio::{Output, PinDriver};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{gpio::AnyOutputPin, modem::Modem, prelude::Peripherals, rmt::RMT},
    http::client::{Configuration as HttpClientConfiguration, EspHttpConnection},
    http::server::{Configuration as HttpConfiguration, EspHttpServer},
    ipv4::{
        ClientConfiguration as IpClientConfiguration, ClientSettings as IpClientSettings,
        Configuration as IpConfiguration, Mask, Subnet,
    },
    log::EspLogger,
    mqtt::client::{EspMqttClient, EspMqttConnection, MqttClientConfiguration},
    netif::{EspNetif, NetifConfiguration},
    nvs::{EspDefaultNvsPartition, EspNvs},
    ota::EspOta,
    sntp::EspSntp,
    wifi::{BlockingWifi, EspWifi},
};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use thermostat_common::{
    config::{IrHardwareConfig, NetworkConfig},
    DayOfWeek, EngineAction, PersistedSettings, RuntimeConfig, Schedule, ScheduleAction,
    ScheduleEntry, ThermostatConfig, ThermostatEngine, ThermostatMode, TOPIC_CMD_HOLD,
    TOPIC_CMD_MODE, TOPIC_CMD_POWER, TOPIC_CMD_SCHEDULE, TOPIC_CMD_TARGET,
    TOPIC_CONTROLLER_SCHEDULE_STATE, TOPIC_CONTROLLER_STATE, TOPIC_SENSOR_HUMIDITY,
    TOPIC_SENSOR_TEMP,
};

use crate::ir::IrTransmitter;

const NVS_NAMESPACE: &str = "thermostat";
const NVS_RUNTIME_KEY: &str = "runtime_json";
const NVS_SCHEDULE_KEY: &str = "schedule_json";
const MAX_HTTP_BODY: usize = 4096;
const OTA_CHUNK_SIZE: usize = 4096;
const MAX_MQTT_PAYLOAD_BYTES: usize = 512;
const PROVISIONING_AP_SSID: &str = "ThermostatController-AP";
const PROVISIONING_AP_PASSWORD: &str = "ThermostatSetup";
const WATCHDOG_TIMEOUT_SEC: u32 = 30;
const SETTINGS_SAVE_RETRY_MS: u64 = 1_000;
const WIFI_RESTART_GRACE_MS: u64 = 300_000;
const WIFI_CONNECT_ATTEMPTS: u32 = 5;
const WIFI_RETRY_DELAY_MS: u64 = 3_000;
const STATUS_LED_PIN: i32 = 48;
const LED_FAST_BLINK_MS: u64 = 200;
const LED_SLOW_BLINK_MS: u64 = 900;

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE_CSS: &str = include_str!("../web/style.css");
const PROVISIONING_INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Thermostat Provisioning</title>
  <style>
    body{font-family:Arial,sans-serif;max-width:720px;margin:2rem auto;padding:0 1rem;color:#111}
    h1{margin:0 0 .5rem}.card{border:1px solid #ddd;border-radius:8px;padding:1rem}
    label{display:block;margin:.5rem 0 .2rem}input[type=text],input[type=password],input[type=number]{width:100%;padding:.5rem;box-sizing:border-box}
    .row{display:flex;gap:1rem}.row>div{flex:1}.muted{color:#555}.ok{color:#106010}.err{color:#a00000}
    button{padding:.55rem .9rem;margin-top:.8rem}
  </style>
</head>
<body>
  <h1>Thermostat Provisioning</h1>
  <p class="muted">Update WiFi and MQTT settings, then restart the device.</p>
  <p class="muted">Provisioning AP password: <code>ThermostatSetup</code></p>
  <div class="card">
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
      <div><label>Static IP</label><input id="staticIp" type="text" placeholder="192.168.1.50"></div>
      <div><label>Gateway</label><input id="gateway" type="text" placeholder="192.168.1.1"></div>
    </div>
    <div class="row">
      <div><label>Subnet Mask</label><input id="subnet" type="text" placeholder="255.255.255.0"></div>
      <div><label>DNS</label><input id="dns" type="text" placeholder="192.168.1.1"></div>
    </div>
    <button id="save">Save Configuration</button>
    <button id="restart">Restart Device</button>
    <div id="status" class="muted"></div>
  </div>
  <script>
    const q=(id)=>document.getElementById(id);
    const toStr=(arr)=>Array.isArray(arr)?arr.join('.'):'';
    const toArr=(value)=>{if(!value.trim())return null;const p=value.trim().split('.').map(Number);if(p.length!==4||p.some(n=>!Number.isInteger(n)||n<0||n>255))throw new Error('Invalid IPv4: '+value);return p;};
    async function api(path,opt){const r=await fetch(path,opt);let b={};try{b=await r.json();}catch(_){}if(!r.ok)throw new Error(b.error||('Request failed: '+r.status));return b;}
    async function load(){
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
    load().catch((err)=>{q('status').className='err';q('status').textContent=err.message;});
  </script>
</body>
</html>
"#;

enum WifiStartup {
    Connected(EspWifi<'static>),
    Provisioning(EspWifi<'static>),
}

#[derive(Clone)]
struct SharedState {
    engine: Arc<Mutex<ThermostatEngine>>,
    schedule: Arc<Mutex<Schedule>>,
    timezone: Arc<Mutex<String>>,
    time_synced: Arc<AtomicBool>,
    ir_sender: Arc<Mutex<IrTransmitter>>,
    ota: Arc<Mutex<OtaRuntimeState>>,
    settings_save_deadline_ms: Arc<Mutex<Option<u64>>>,
    wifi_connected: Arc<AtomicBool>,
    mqtt_connected: Arc<AtomicBool>,
}

struct StatusLed {
    pin: PinDriver<'static, AnyOutputPin, Output>,
    lit: bool,
}

#[derive(Clone)]
struct NvsStore {
    partition: EspDefaultNvsPartition,
    lock: Arc<Mutex<()>>,
}

#[derive(Debug, Serialize)]
struct TimeStatus {
    #[serde(rename = "timeSynced")]
    time_synced: bool,
    timezone: String,
    #[serde(rename = "nowEpoch")]
    now_epoch: i64,
}

#[derive(Debug, Deserialize)]
struct TimezoneUpdate {
    timezone: String,
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

#[derive(Debug, Serialize)]
struct IrConfigView {
    #[serde(rename = "txPin")]
    tx_pin: i32,
    #[serde(rename = "rmtChannel")]
    rmt_channel: u8,
    #[serde(rename = "carrierKHz")]
    carrier_khz: u32,
}

#[derive(Debug, Deserialize)]
struct IrConfigUpdate {
    #[serde(rename = "txPin")]
    tx_pin: i32,
    #[serde(rename = "rmtChannel")]
    rmt_channel: u8,
    #[serde(rename = "carrierKHz")]
    carrier_khz: u32,
}

#[derive(Debug, Serialize)]
struct IrConfigUpdateResponse {
    #[serde(rename = "restartRequired")]
    restart_required: bool,
    ir: IrConfigView,
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

    runtime.settings.sanitize();
    runtime.ir.sanitize();
    ensure_wifi_defaults(&mut runtime);

    info!(
        "NVS config loaded: ssid=`{}`, static_ip={}, mqtt=`{}:{}`",
        runtime.network.wifi_ssid,
        runtime.network.use_static_ip,
        runtime.network.mqtt_host,
        runtime.network.mqtt_port,
    );

    let Peripherals { modem, rmt, .. } = Peripherals::take()?;
    let ir_sender = match init_ir_transmitter(rmt, &runtime.ir) {
        Ok(transmitter) => {
            info!(
                "IR transmitter initialized on RMT channel{} / GPIO{} @ {}kHz",
                runtime.ir.rmt_channel, runtime.ir.tx_pin, runtime.ir.carrier_khz
            );
            transmitter
        }
        Err(err) => {
            warn!("failed to initialize IR transmitter, running disabled: {err:#}");
            IrTransmitter::disabled()
        }
    };

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

    let _sntp = EspSntp::new_default().context("failed to start SNTP")?;
    info!("SNTP initialized");

    init_watchdog(WATCHDOG_TIMEOUT_SEC)?;

    if let Ok(mut ota) = EspOta::new() {
        if let Err(err) = ota.mark_running_slot_valid() {
            warn!("failed to mark running OTA slot valid: {err:?}");
        }
    }

    let schedule = nvs_store.load_schedule().unwrap_or_else(|err| {
        warn!("failed to load schedule from NVS: {err:#}");
        Schedule::default()
    });

    let shared_state = SharedState {
        engine: Arc::new(Mutex::new(ThermostatEngine::new(
            ThermostatConfig::default(),
            runtime.settings.clone(),
        ))),
        schedule: Arc::new(Mutex::new(schedule)),
        timezone: Arc::new(Mutex::new(runtime.timezone.clone())),
        time_synced: Arc::new(AtomicBool::new(false)),
        ir_sender: Arc::new(Mutex::new(ir_sender)),
        ota: Arc::new(Mutex::new(OtaRuntimeState::default())),
        settings_save_deadline_ms: Arc::new(Mutex::new(None)),
        wifi_connected: Arc::new(AtomicBool::new(true)),
        mqtt_connected: Arc::new(AtomicBool::new(false)),
    };
    let status_led = init_status_led(STATUS_LED_PIN);

    let (mqtt_client, mqtt_conn) = create_mqtt_client(&runtime.network)?;
    let mqtt_client = Arc::new(Mutex::new(mqtt_client));

    subscribe_topics(&mqtt_client)?;
    spawn_mqtt_receiver(
        shared_state.clone(),
        nvs_store.clone(),
        mqtt_conn,
        mqtt_client.clone(),
    );
    spawn_control_loop(
        shared_state.clone(),
        nvs_store.clone(),
        mqtt_client.clone(),
        status_led,
    );

    let server = create_http_server(shared_state.clone(), nvs_store)?;

    // Keep services alive for the program lifetime.
    let _wifi = wifi;
    let _server = server;

    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

fn ensure_wifi_defaults(runtime: &mut RuntimeConfig) {
    if runtime.network.wifi_ssid.is_empty() {
        if let Some(ssid) = option_env!("WIFI_SSID") {
            runtime.network.wifi_ssid = ssid.to_string();
        }
    }

    if runtime.network.wifi_pass.is_empty() {
        if let Some(pass) = option_env!("WIFI_PASS") {
            runtime.network.wifi_pass = pass.to_string();
        }
    }
}

fn create_http_server(
    state: SharedState,
    nvs_store: NvsStore,
) -> anyhow::Result<EspHttpServer<'static>> {
    let conf = HttpConfiguration {
        stack_size: 16 * 1024,
        ..Default::default()
    };

    let mut server = EspHttpServer::new(&conf)?;

    server.fn_handler::<anyhow::Error, _>("/", Method::Get, move |req| {
        req.into_ok_response()?.write_all(INDEX_HTML.as_bytes())?;
        Ok(())
    })?;

    server.fn_handler::<anyhow::Error, _>("/app.js", Method::Get, move |req| {
        req.into_ok_response()?.write_all(APP_JS.as_bytes())?;
        Ok(())
    })?;

    server.fn_handler::<anyhow::Error, _>("/style.css", Method::Get, move |req| {
        req.into_ok_response()?.write_all(STYLE_CSS.as_bytes())?;
        Ok(())
    })?;

    {
        let state = state.clone();
        server.fn_handler("/api/status", Method::Get, move |req| {
            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/target", Method::Post, move |req| {
            let uri = req.uri().to_string();
            let Some(target) =
                query_param(&uri, "value").and_then(|value| value.parse::<f32>().ok())
            else {
                return write_error(req, 400, "Missing or invalid 'value' parameter");
            };

            let now_ms = monotonic_ms();
            {
                let mut engine = state.engine.lock().unwrap();
                let changed = engine.set_target_temp(target);
                if changed {
                    queue_settings_save(&state, now_ms, engine.config.settings_save_debounce_ms);
                }
            }

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/mode", Method::Post, move |req| {
            let uri = req.uri().to_string();
            let Some(value) = query_param(&uri, "value") else {
                return write_error(req, 400, "Missing 'value' parameter");
            };

            let mode = match value.to_ascii_uppercase().as_str() {
                "HEAT" => ThermostatMode::Heat,
                "OFF" => ThermostatMode::Off,
                _ => return write_error(req, 400, "Invalid mode. Use 'HEAT' or 'OFF'"),
            };

            let now_ms = monotonic_ms();
            let (changed, actions, debounce_ms) = {
                let mut engine = state.engine.lock().unwrap();
                let debounce_ms = engine.config.settings_save_debounce_ms;
                let (changed, actions) = engine.set_mode_with_actions(mode, now_ms);
                (changed, actions, debounce_ms)
            };
            execute_engine_actions(&state, actions);
            if changed {
                queue_settings_save(&state, now_ms, debounce_ms);
            }

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/hysteresis", Method::Post, move |req| {
            let uri = req.uri().to_string();
            let Some(value) = query_param(&uri, "value") else {
                return write_error(req, 400, "Missing 'value' parameter");
            };
            let Ok(hysteresis) = value.parse::<f32>() else {
                return write_error(req, 400, "Invalid hysteresis value (0.5-5.0)");
            };

            if !(0.5..=5.0).contains(&hysteresis) {
                return write_error(req, 400, "Invalid hysteresis value (0.5-5.0)");
            }

            let now_ms = monotonic_ms();
            {
                let mut engine = state.engine.lock().unwrap();
                let changed = engine.set_hysteresis(hysteresis);
                if changed {
                    queue_settings_save(&state, now_ms, engine.config.settings_save_debounce_ms);
                }
            }

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/offset", Method::Post, move |req| {
            let uri = req.uri().to_string();
            let Some(value) = query_param(&uri, "value") else {
                return write_error(req, 400, "Missing 'value' parameter");
            };
            let Ok(offset) = value.parse::<i32>() else {
                return write_error(req, 400, "Invalid offset value (2-10, even only)");
            };

            if !(2..=10).contains(&offset) || offset % 2 != 0 {
                return write_error(req, 400, "Invalid offset value (2-10, even only)");
            }

            let now_ms = monotonic_ms();
            {
                let mut engine = state.engine.lock().unwrap();
                let changed = engine.set_fireplace_offset(offset);
                if changed {
                    queue_settings_save(&state, now_ms, engine.config.settings_save_debounce_ms);
                }
            }

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/ir/on", Method::Post, move |req| {
            let actions = {
                let mut engine = state.engine.lock().unwrap();
                engine.manual_on(monotonic_ms())
            };
            execute_engine_actions(&state, actions);

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/ir/off", Method::Post, move |req| {
            let actions = {
                let mut engine = state.engine.lock().unwrap();
                engine.manual_off(monotonic_ms())
            };
            execute_engine_actions(&state, actions);

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/ir/heat/on", Method::Post, move |req| {
            let actions = {
                let mut engine = state.engine.lock().unwrap();
                engine.manual_heat_on(monotonic_ms())
            };
            execute_engine_actions(&state, actions);

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/ir/heat/off", Method::Post, move |req| {
            let actions = {
                let mut engine = state.engine.lock().unwrap();
                engine.manual_heat_off(monotonic_ms())
            };
            execute_engine_actions(&state, actions);

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/ir/heat/up", Method::Post, move |req| {
            let actions = {
                let mut engine = state.engine.lock().unwrap();
                engine.manual_heat_up()
            };
            execute_engine_actions(&state, actions);

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/ir/heat/down", Method::Post, move |req| {
            let actions = {
                let mut engine = state.engine.lock().unwrap();
                engine.manual_heat_down()
            };
            execute_engine_actions(&state, actions);

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/ir/light/toggle", Method::Post, move |req| {
            let actions = {
                let mut engine = state.engine.lock().unwrap();
                engine.manual_light_toggle()
            };
            execute_engine_actions(&state, actions);

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/ir/timer/toggle", Method::Post, move |req| {
            let actions = {
                let mut engine = state.engine.lock().unwrap();
                engine.manual_timer_toggle()
            };
            execute_engine_actions(&state, actions);

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/hold/enter", Method::Post, move |req| {
            let uri = req.uri().to_string();
            let duration_ms = query_param(&uri, "minutes")
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
                .map(|value| value * 60_000);

            {
                let mut engine = state.engine.lock().unwrap();
                engine.enter_hold(duration_ms, monotonic_ms());
            }

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/hold/exit", Method::Post, move |req| {
            {
                let mut engine = state.engine.lock().unwrap();
                engine.exit_hold();
            }

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/safety/reset", Method::Post, move |req| {
            {
                let mut engine = state.engine.lock().unwrap();
                engine.reset_safety();
            }

            let status = build_status(&state);
            write_json(req, &status)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/schedule", Method::Get, move |req| {
            let schedule = state.schedule.lock().unwrap().clone();
            write_json(req, &schedule)
        })?;
    }

    {
        let state = state.clone();
        let nvs_store = nvs_store.clone();
        server.fn_handler::<anyhow::Error, _>("/api/schedule", Method::Put, move |mut req| {
            let body = read_request_body(&mut req)?;
            let mut schedule: Schedule =
                serde_json::from_slice(&body).context("invalid schedule payload")?;
            schedule.normalize();

            {
                let mut current = state.schedule.lock().unwrap();
                *current = schedule.clone();
            }

            nvs_store.save_schedule(&schedule)?;
            write_json(req, &schedule)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/time", Method::Get, move |req| {
            let timezone = state.timezone.lock().unwrap().clone();
            let payload = TimeStatus {
                time_synced: state.time_synced.load(Ordering::Relaxed),
                timezone,
                now_epoch: Utc::now().timestamp(),
            };

            write_json(req, &payload)
        })?;
    }

    {
        let state = state.clone();
        let nvs_store = nvs_store.clone();
        server.fn_handler::<anyhow::Error, _>("/api/timezone", Method::Put, move |mut req| {
            let body = read_request_body(&mut req)?;
            let update: TimezoneUpdate =
                serde_json::from_slice(&body).context("invalid timezone payload")?;

            if update.timezone.parse::<Tz>().is_err() {
                return write_error(req, 400, "Invalid timezone value");
            }

            {
                let mut timezone = state.timezone.lock().unwrap();
                *timezone = update.timezone.clone();
            }

            persist_runtime_from_state(&nvs_store, &state)?;

            let payload = TimeStatus {
                time_synced: state.time_synced.load(Ordering::Relaxed),
                timezone: update.timezone,
                now_epoch: Utc::now().timestamp(),
            };

            write_json(req, &payload)
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

    {
        let nvs_store = nvs_store.clone();
        server.fn_handler("/api/ir/config", Method::Get, move |req| {
            let runtime = nvs_store.load_runtime_config().unwrap_or_default();
            let payload = build_ir_config_view(&runtime.ir);
            write_json(req, &payload)
        })?;
    }

    {
        let nvs_store = nvs_store.clone();
        server.fn_handler::<anyhow::Error, _>("/api/ir/config", Method::Put, move |mut req| {
            let body = read_request_body(&mut req)?;
            let update: IrConfigUpdate =
                serde_json::from_slice(&body).context("invalid ir config payload")?;

            if let Err(message) = validate_ir_update(&update) {
                return write_error(req, 400, message);
            }

            let payload = apply_ir_update(&nvs_store, update)?;
            write_json(req, &payload)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/ir/diagnostics", Method::Get, move |req| {
            let diagnostics = state.ir_sender.lock().unwrap().diagnostics();
            write_json(req, &diagnostics)
        })?;
    }

    {
        let state = state.clone();
        server.fn_handler("/api/ota/status", Method::Get, move |req| {
            let payload = build_ota_status_response(&state);
            write_json(req, &payload)
        })?;
    }

    {
        let state = state.clone();
        let nvs_store = nvs_store.clone();
        server.fn_handler::<anyhow::Error, _>("/api/ota/apply", Method::Post, move |mut req| {
            let body = read_request_body(&mut req)?;
            let update: OtaApplyRequest =
                serde_json::from_slice(&body).context("invalid ota payload")?;

            if let Err(message) = validate_ota_apply_request(&update) {
                return write_error(req, 400, message);
            }

            match apply_ota_update(&state, &nvs_store, update) {
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
            req.into_ok_response()?
                .write_all(PROVISIONING_INDEX_HTML.as_bytes())?;
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
            // Auto-restart so user doesn't need to click "Restart Device" separately
            thread::Builder::new()
                .name("prov-restart".into())
                .spawn(|| {
                    thread::sleep(Duration::from_secs(3));
                    unsafe { esp_idf_svc::sys::esp_restart() };
                })
                .expect("failed to spawn restart thread");
            write_json(req, &payload)
        })?;
    }

    {
        let nvs_store = nvs_store.clone();
        server.fn_handler("/api/ir/config", Method::Get, move |req| {
            let runtime = nvs_store.load_runtime_config().unwrap_or_default();
            let payload = build_ir_config_view(&runtime.ir);
            write_json(req, &payload)
        })?;
    }

    {
        let nvs_store = nvs_store.clone();
        server.fn_handler::<anyhow::Error, _>("/api/ir/config", Method::Put, move |mut req| {
            let body = read_request_body(&mut req)?;
            let update: IrConfigUpdate =
                serde_json::from_slice(&body).context("invalid ir config payload")?;

            if let Err(message) = validate_ir_update(&update) {
                return write_error(req, 400, message);
            }

            let payload = apply_ir_update(&nvs_store, update)?;
            write_json(req, &payload)
        })?;
    }

    {
        server.fn_handler("/api/ota/status", Method::Get, move |req| {
            let payload = OtaStatusResponse {
                supported: false,
                in_progress: false,
                bytes_written: 0,
                total_bytes: None,
                progress_pct: None,
                last_error: Some(
                    "Controller is in provisioning mode; OTA apply is unavailable".to_string(),
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
    }

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

fn query_param(uri: &str, key: &str) -> Option<String> {
    let query = uri.split_once('?')?.1;
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let name = parts.next()?;
        let value = parts.next().unwrap_or_default();
        if name == key {
            return Some(value.replace('+', " "));
        }
    }

    None
}

fn init_ir_transmitter(rmt: RMT, ir: &IrHardwareConfig) -> anyhow::Result<IrTransmitter> {
    if ir.tx_pin < 0 {
        return Err(anyhow!("invalid tx pin: {}", ir.tx_pin));
    }

    let pin = ir.tx_pin;
    let carrier_khz = ir.carrier_khz;

    match ir.rmt_channel {
        0 => unsafe {
            IrTransmitter::new_with_carrier(rmt.channel0, AnyOutputPin::new(pin), carrier_khz)
        },
        1 => unsafe {
            IrTransmitter::new_with_carrier(rmt.channel1, AnyOutputPin::new(pin), carrier_khz)
        },
        2 => unsafe {
            IrTransmitter::new_with_carrier(rmt.channel2, AnyOutputPin::new(pin), carrier_khz)
        },
        3 => unsafe {
            IrTransmitter::new_with_carrier(rmt.channel3, AnyOutputPin::new(pin), carrier_khz)
        },
        #[cfg(any(esp32, esp32s3))]
        4 => unsafe {
            IrTransmitter::new_with_carrier(rmt.channel4, AnyOutputPin::new(pin), carrier_khz)
        },
        #[cfg(any(esp32, esp32s3))]
        5 => unsafe {
            IrTransmitter::new_with_carrier(rmt.channel5, AnyOutputPin::new(pin), carrier_khz)
        },
        #[cfg(any(esp32, esp32s3))]
        6 => unsafe {
            IrTransmitter::new_with_carrier(rmt.channel6, AnyOutputPin::new(pin), carrier_khz)
        },
        #[cfg(any(esp32, esp32s3))]
        7 => unsafe {
            IrTransmitter::new_with_carrier(rmt.channel7, AnyOutputPin::new(pin), carrier_khz)
        },
        _ => Err(anyhow!("unsupported RMT channel: {}", ir.rmt_channel)),
    }
}

fn has_station_credentials(network: &NetworkConfig) -> bool {
    let ssid = network.wifi_ssid.trim();
    !ssid.is_empty() && ssid != "CHANGE_ME"
}

fn ipv4_from_octets(ip: [u8; 4]) -> Ipv4Addr {
    Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3])
}

fn build_sta_netif(network: &NetworkConfig) -> anyhow::Result<Option<EspNetif>> {
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

    let conf = NetifConfiguration {
        ip_configuration: Some(IpConfiguration::Client(IpClientConfiguration::Fixed(
            IpClientSettings {
                ip: ipv4_from_octets(static_ip),
                subnet: Subnet {
                    gateway: ipv4_from_octets(gateway),
                    mask,
                },
                dns: network.dns.map(ipv4_from_octets),
                secondary_dns: None,
            },
        ))),
        ..NetifConfiguration::wifi_default_client()
    };

    Ok(Some(EspNetif::new_with_conf(&conf)?))
}

fn connect_wifi(
    modem: Modem,
    sys_loop: EspSystemEventLoop,
    nvs_partition: EspDefaultNvsPartition,
    network: &thermostat_common::config::NetworkConfig,
) -> anyhow::Result<WifiStartup> {
    let mut esp_wifi = EspWifi::new(modem, sys_loop.clone(), Some(nvs_partition))?;

    let static_ip_error = match build_sta_netif(network) {
        Ok(Some(sta_netif)) => {
            esp_wifi
                .swap_netif_sta(sta_netif)
                .context("failed to apply static IP netif configuration")?;
            None
        }
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
        auth_method: AuthMethod::WPA2Personal,
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

fn create_mqtt_client(
    network: &thermostat_common::config::NetworkConfig,
) -> anyhow::Result<(EspMqttClient<'static>, EspMqttConnection)> {
    let url = format!("mqtt://{}:{}", network.mqtt_host, network.mqtt_port);

    let conf = MqttClientConfiguration {
        client_id: Some("thermostat-controller"),
        username: if network.mqtt_user.is_empty() {
            None
        } else {
            Some(network.mqtt_user.as_str())
        },
        password: if network.mqtt_pass.is_empty() {
            None
        } else {
            Some(network.mqtt_pass.as_str())
        },
        ..Default::default()
    };

    Ok(EspMqttClient::new(url.as_str(), &conf)?)
}

fn subscribe_topics(mqtt: &Arc<Mutex<EspMqttClient<'static>>>) -> anyhow::Result<()> {
    let topics = [
        TOPIC_SENSOR_TEMP,
        TOPIC_SENSOR_HUMIDITY,
        TOPIC_CMD_POWER,
        TOPIC_CMD_TARGET,
        TOPIC_CMD_MODE,
        TOPIC_CMD_HOLD,
        TOPIC_CMD_SCHEDULE,
    ];

    let mut mqtt = mqtt.lock().unwrap();
    for topic in topics {
        mqtt.subscribe(topic, QoS::AtMostOnce)?;
    }

    Ok(())
}

fn spawn_mqtt_receiver(
    state: SharedState,
    nvs_store: NvsStore,
    mut conn: EspMqttConnection,
    mqtt: Arc<Mutex<EspMqttClient<'static>>>,
) {
    thread::Builder::new()
        .name("mqtt-rx".into())
        .stack_size(12 * 1024)
        .spawn(move || {
            loop {
                match conn.next() {
                    Ok(event) => {
                        state.mqtt_connected.store(true, Ordering::Relaxed);

                        if let EventPayload::Received {
                            topic: Some(topic),
                            data,
                            details,
                            ..
                        } = event.payload()
                        {
                            // We only process full MQTT payloads.
                            if !matches!(details, Details::Complete) {
                                continue;
                            }

                            if data.len() > MAX_MQTT_PAYLOAD_BYTES {
                                warn!(
                                    "dropping oversized MQTT payload on topic {} ({} bytes)",
                                    topic,
                                    data.len()
                                );
                                continue;
                            }

                            if let Ok(message) = core::str::from_utf8(data) {
                                if let Err(err) =
                                    handle_mqtt_message(&state, &nvs_store, topic, message)
                                {
                                    warn!("mqtt message handling failed: {err:#}");
                                }
                            }
                        }
                    }
                    Err(err) => {
                        state.mqtt_connected.store(false, Ordering::Relaxed);
                        warn!("mqtt receive loop error: {err:?}");
                        thread::sleep(Duration::from_secs(2));
                        if let Err(sub_err) = subscribe_topics(&mqtt) {
                            warn!("mqtt re-subscribe failed: {sub_err:#}");
                        }
                    }
                }
            }
        })
        .expect("failed to spawn mqtt receiver thread");
}

fn spawn_control_loop(
    state: SharedState,
    nvs_store: NvsStore,
    mqtt: Arc<Mutex<EspMqttClient<'static>>>,
    mut status_led: Option<StatusLed>,
) {
    thread::Builder::new()
        .name("control-loop".into())
        .stack_size(12 * 1024)
        .spawn(move || {
            if let Err(err) = add_current_task_to_watchdog() {
                warn!("failed to register control loop with watchdog: {err:#}");
            }

            let mut last_state_publish_ms = 0_u64;
            let mut wifi_disconnected_since_ms: Option<u64> = None;

            loop {
                feed_watchdog();
                let now_ms = monotonic_ms();
                let wifi_connected = is_wifi_station_connected();
                let mqtt_connected = state.mqtt_connected.load(Ordering::Relaxed);

                state
                    .wifi_connected
                    .store(wifi_connected, Ordering::Relaxed);
                update_status_led(&mut status_led, wifi_connected, mqtt_connected, now_ms);

                if wifi_connected {
                    wifi_disconnected_since_ms = None;
                } else if let Some(disconnected_since_ms) = wifi_disconnected_since_ms {
                    if now_ms.saturating_sub(disconnected_since_ms) >= WIFI_RESTART_GRACE_MS {
                        warn!(
                            "wifi disconnected for {}s; restarting device for recovery",
                            WIFI_RESTART_GRACE_MS / 1000
                        );
                        thread::sleep(Duration::from_millis(100));
                        unsafe { esp_idf_svc::sys::esp_restart() };
                    }
                } else {
                    wifi_disconnected_since_ms = Some(now_ms);
                }

                let timezone = state.timezone.lock().unwrap().clone();
                let now_in_tz = now_in_timezone(&timezone);
                state
                    .time_synced
                    .store(now_in_tz.is_some(), Ordering::Relaxed);

                if let Some(now) = now_in_tz {
                    let schedule_action = {
                        let schedule = state.schedule.lock().unwrap();
                        schedule.current_action(now)
                    };

                    if let Some(ScheduleAction {
                        mode,
                        target_temp_f,
                    }) = schedule_action
                    {
                        let mut engine = state.engine.lock().unwrap();
                        let (_, schedule_actions) =
                            engine.apply_schedule_action(mode, target_temp_f, now_ms);
                        drop(engine);
                        execute_engine_actions(&state, schedule_actions);
                    }
                }

                let actions = {
                    let mut engine = state.engine.lock().unwrap();
                    engine.tick(now_ms)
                };

                execute_engine_actions(&state, actions);
                flush_pending_settings_save(&nvs_store, &state, now_ms);

                if now_ms.saturating_sub(last_state_publish_ms) >= 10_000 {
                    last_state_publish_ms = now_ms;
                    if let Err(err) = publish_state(&state, &mqtt) {
                        warn!("state publish failed: {err:#}");
                    }
                }

                thread::sleep(Duration::from_millis(200));
            }
        })
        .expect("failed to spawn control loop thread");
}

fn publish_state(
    state: &SharedState,
    mqtt: &Arc<Mutex<EspMqttClient<'static>>>,
) -> anyhow::Result<()> {
    let now_ms = monotonic_ms();

    let payload = {
        let engine = state.engine.lock().unwrap();
        serde_json::to_vec(&engine.state_payload(now_ms))?
    };

    {
        let mut client = mqtt.lock().unwrap();
        client.publish(TOPIC_CONTROLLER_STATE, QoS::AtLeastOnce, true, &payload)?;
    }

    let schedule_payload = {
        let schedule = state.schedule.lock().unwrap().clone();
        serde_json::to_vec(&schedule)?
    };

    {
        let mut client = mqtt.lock().unwrap();
        client.publish(
            TOPIC_CONTROLLER_SCHEDULE_STATE,
            QoS::AtLeastOnce,
            true,
            &schedule_payload,
        )?;
    }

    Ok(())
}

fn handle_mqtt_message(
    state: &SharedState,
    nvs_store: &NvsStore,
    topic: &str,
    message: &str,
) -> anyhow::Result<()> {
    let now_ms = monotonic_ms();

    match topic {
        TOPIC_SENSOR_TEMP => {
            if let Ok(temp) = message.parse::<f32>() {
                if temp.is_finite() && (-40.0..=150.0).contains(&temp) {
                    let mut engine = state.engine.lock().unwrap();
                    let humidity = engine.current_humidity();
                    engine.update_sensor_data(temp, humidity, now_ms);
                }
            }
        }
        TOPIC_SENSOR_HUMIDITY => {
            if let Ok(humidity) = message.parse::<f32>() {
                if humidity.is_finite() && (0.0..=100.0).contains(&humidity) {
                    let mut engine = state.engine.lock().unwrap();
                    let temp = engine.current_temp_f();
                    engine.update_sensor_data(temp, humidity, now_ms);
                }
            }
        }
        TOPIC_CMD_POWER => {
            let actions = {
                let mut engine = state.engine.lock().unwrap();
                if message.eq_ignore_ascii_case("on") {
                    engine.manual_on(now_ms)
                } else if message.eq_ignore_ascii_case("off") {
                    engine.manual_off(now_ms)
                } else {
                    Vec::new()
                }
            };
            execute_engine_actions(state, actions);
        }
        TOPIC_CMD_TARGET => {
            if let Ok(target) = message.parse::<f32>() {
                let mut engine = state.engine.lock().unwrap();
                let changed = engine.set_target_temp(target);
                if changed {
                    queue_settings_save(state, now_ms, engine.config.settings_save_debounce_ms);
                }
            }
        }
        TOPIC_CMD_MODE => {
            let (changed, actions, debounce_ms) = {
                let mut engine = state.engine.lock().unwrap();
                let debounce_ms = engine.config.settings_save_debounce_ms;
                if message.eq_ignore_ascii_case("HEAT") {
                    let (changed, actions) =
                        engine.set_mode_with_actions(ThermostatMode::Heat, now_ms);
                    (changed, actions, debounce_ms)
                } else if message.eq_ignore_ascii_case("OFF") {
                    let (changed, actions) =
                        engine.set_mode_with_actions(ThermostatMode::Off, now_ms);
                    (changed, actions, debounce_ms)
                } else {
                    (false, Vec::new(), debounce_ms)
                }
            };
            execute_engine_actions(state, actions);
            if changed {
                queue_settings_save(state, now_ms, debounce_ms);
            }
        }
        TOPIC_CMD_HOLD => {
            let mut engine = state.engine.lock().unwrap();
            if message.eq_ignore_ascii_case("on") || message.eq_ignore_ascii_case("enter") {
                engine.enter_hold(None, now_ms);
            } else if message.eq_ignore_ascii_case("off") || message.eq_ignore_ascii_case("exit") {
                engine.exit_hold();
            } else if let Ok(minutes) = message.parse::<u64>() {
                if minutes > 0 && minutes <= engine.config.max_hold_minutes as u64 {
                    engine.enter_hold(Some(minutes * 60_000), now_ms);
                }
            }
        }
        TOPIC_CMD_SCHEDULE => {
            if let Ok(mut schedule) = serde_json::from_str::<Schedule>(message) {
                schedule.normalize();
                {
                    let mut current = state.schedule.lock().unwrap();
                    *current = schedule.clone();
                }
                nvs_store.save_schedule(&schedule)?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn execute_engine_actions(state: &SharedState, actions: Vec<EngineAction>) {
    for action in actions {
        if let EngineAction::Delay(ms) = action {
            thread::sleep(Duration::from_millis(ms));
            continue;
        }

        let mut transmitter = state.ir_sender.lock().unwrap();
        let description = format!("{action:?}");
        if let Err(err) = transmitter.execute_action(action) {
            warn!("engine action failed [{description}]: {err:#}");
        } else {
            info!("engine action sent [{description}]");
        }
    }
}

fn build_status(state: &SharedState) -> thermostat_common::ControllerStatus {
    let now_ms = monotonic_ms();
    let timezone = state.timezone.lock().unwrap().clone();
    let time_synced = state.time_synced.load(Ordering::Relaxed);

    let next_schedule_event_epoch = {
        let schedule = state.schedule.lock().unwrap();
        now_in_timezone(&timezone).and_then(|now| schedule.next_event_epoch(now))
    };

    let schedule_enabled = state.schedule.lock().unwrap().enabled;

    let engine = state.engine.lock().unwrap();
    engine.status(
        now_ms,
        schedule_enabled,
        next_schedule_event_epoch,
        time_synced,
        &timezone,
    )
}

fn persist_runtime_from_state(nvs_store: &NvsStore, state: &SharedState) -> anyhow::Result<()> {
    let settings = state.engine.lock().unwrap().settings().clone();
    persist_runtime(nvs_store, state, &settings)
}

fn persist_runtime(
    nvs_store: &NvsStore,
    state: &SharedState,
    settings: &PersistedSettings,
) -> anyhow::Result<()> {
    let mut runtime = nvs_store.load_runtime_config().unwrap_or_default();
    runtime.settings = settings.clone();
    runtime.timezone = state.timezone.lock().unwrap().clone();
    nvs_store.save_runtime_config(&runtime)
}

fn queue_settings_save(state: &SharedState, now_ms: u64, debounce_ms: u64) {
    let mut deadline = state.settings_save_deadline_ms.lock().unwrap();
    *deadline = Some(now_ms.saturating_add(debounce_ms.max(250)));
}

fn flush_pending_settings_save(nvs_store: &NvsStore, state: &SharedState, now_ms: u64) {
    let should_persist = {
        let mut deadline = state.settings_save_deadline_ms.lock().unwrap();
        match *deadline {
            Some(due_ms) if now_ms >= due_ms => {
                *deadline = None;
                true
            }
            _ => false,
        }
    };

    if !should_persist {
        return;
    }

    if let Err(err) = persist_runtime_from_state(nvs_store, state) {
        warn!("failed to persist debounced runtime settings: {err:#}");
        queue_settings_save(state, now_ms, SETTINGS_SAVE_RETRY_MS);
    }
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

fn validate_ir_update(update: &IrConfigUpdate) -> Result<(), &'static str> {
    if update.tx_pin < 0 {
        return Err("txPin must be >= 0");
    }
    if !is_supported_rmt_channel(update.rmt_channel) {
        return Err("rmtChannel is not supported on this target");
    }
    if !(10..=100).contains(&update.carrier_khz) {
        return Err("carrierKHz must be between 10 and 100");
    }

    Ok(())
}

fn apply_ir_update(
    nvs_store: &NvsStore,
    update: IrConfigUpdate,
) -> anyhow::Result<IrConfigUpdateResponse> {
    let mut runtime = nvs_store.load_runtime_config().unwrap_or_default();
    let previous = runtime.ir.clone();

    runtime.ir.tx_pin = update.tx_pin;
    runtime.ir.rmt_channel = update.rmt_channel;
    runtime.ir.carrier_khz = update.carrier_khz;
    runtime.ir.sanitize();

    nvs_store.save_runtime_config(&runtime)?;

    Ok(IrConfigUpdateResponse {
        restart_required: ir_restart_required(&previous, &runtime.ir),
        ir: build_ir_config_view(&runtime.ir),
    })
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
    state: &SharedState,
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
        let mut ota = state.ota.lock().unwrap();
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

    let ota_state = state.ota.clone();
    let spawn_result = thread::Builder::new()
        .name("ota-apply".into())
        .stack_size(16 * 1024)
        .spawn(move || {
            let reboot_after_apply = update.reboot.unwrap_or(true);
            let expected_sha = update
                .sha256
                .as_ref()
                .map(|v| v.trim().to_ascii_lowercase());
            let result = download_and_apply_ota(&ota_state, &update.url, expected_sha.as_deref());

            match result {
                Ok((bytes_written, digest_hex)) => {
                    {
                        let mut ota = ota_state.lock().unwrap();
                        ota.in_progress = false;
                        ota.bytes_written = bytes_written;
                        ota.progress_pct = Some(100);
                        ota.last_error = None;
                        ota.last_sha256 = Some(digest_hex);
                        ota.last_completed_epoch = Some(Utc::now().timestamp());
                    }

                    info!("OTA apply completed successfully ({} bytes)", bytes_written);

                    if reboot_after_apply {
                        thread::sleep(Duration::from_millis(800));
                        unsafe { esp_idf_svc::sys::esp_restart() };
                    }
                }
                Err(err) => {
                    warn!("OTA apply failed: {err:#}");
                    let mut ota = ota_state.lock().unwrap();
                    ota.in_progress = false;
                    ota.last_error = Some(err.to_string());
                    ota.last_completed_epoch = Some(Utc::now().timestamp());
                }
            }
        });

    if let Err(err) = spawn_result {
        let message = format!("failed to spawn OTA apply thread: {err}");
        let mut ota = state.ota.lock().unwrap();
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

fn build_ota_status_response(state: &SharedState) -> OtaStatusResponse {
    let ota = state.ota.lock().unwrap();

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

fn build_ir_config_view(ir: &IrHardwareConfig) -> IrConfigView {
    IrConfigView {
        tx_pin: ir.tx_pin,
        rmt_channel: ir.rmt_channel,
        carrier_khz: ir.carrier_khz,
    }
}

fn ir_restart_required(previous: &IrHardwareConfig, current: &IrHardwareConfig) -> bool {
    previous != current
}

fn is_supported_rmt_channel(channel: u8) -> bool {
    match channel {
        0 | 1 | 2 | 3 => true,
        #[cfg(any(esp32, esp32s3))]
        4 | 5 | 6 | 7 => true,
        _ => false,
    }
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

    fn load_schedule(&self) -> anyhow::Result<Schedule> {
        let _guard = self.lock.lock().unwrap();
        let mut nvs = EspNvs::new(self.partition.clone(), NVS_NAMESPACE, true)?;
        let mut buffer = vec![0_u8; 4096];

        match nvs.get_str(NVS_SCHEDULE_KEY, &mut buffer)? {
            Some(value) => Ok(serde_json::from_str::<Schedule>(value)?),
            None => Ok(Schedule::default()),
        }
    }

    fn save_schedule(&self, schedule: &Schedule) -> anyhow::Result<()> {
        let _guard = self.lock.lock().unwrap();
        let mut nvs = EspNvs::new(self.partition.clone(), NVS_NAMESPACE, true)?;
        let payload = serde_json::to_string(schedule)?;
        nvs.set_str(NVS_SCHEDULE_KEY, &payload)?;
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

fn init_status_led(pin: i32) -> Option<StatusLed> {
    let driver = unsafe { PinDriver::output(AnyOutputPin::new(pin)) };
    match driver {
        Ok(mut pin) => {
            let _ = pin.set_low();
            Some(StatusLed { pin, lit: false })
        }
        Err(err) => {
            warn!("status LED unavailable on GPIO{pin}: {err}");
            None
        }
    }
}

fn update_status_led(
    status_led: &mut Option<StatusLed>,
    wifi_connected: bool,
    mqtt_connected: bool,
    now_ms: u64,
) {
    let desired_on = if !wifi_connected {
        ((now_ms / LED_FAST_BLINK_MS) % 2) == 0
    } else if !mqtt_connected {
        ((now_ms / LED_SLOW_BLINK_MS) % 2) == 0
    } else {
        true
    };

    let Some(led) = status_led.as_mut() else {
        return;
    };

    if desired_on == led.lit {
        return;
    }

    let result = if desired_on {
        led.pin.set_high()
    } else {
        led.pin.set_low()
    };

    if let Err(err) = result {
        warn!("failed to drive status LED: {err}");
    } else {
        led.lit = desired_on;
    }
}

fn now_in_timezone(timezone: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    let tz: Tz = timezone.parse().ok()?;
    let local = Utc::now().with_timezone(&tz);
    Some(local.with_timezone(&local.offset().fix()))
}

fn monotonic_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[allow(dead_code)]
fn _schedule_examples() -> Vec<ScheduleEntry> {
    vec![
        ScheduleEntry {
            day: DayOfWeek::Mon,
            start_minutes: 6 * 60,
            mode: ThermostatMode::Heat,
            target_temp_f: 71.0,
        },
        ScheduleEntry {
            day: DayOfWeek::Mon,
            start_minutes: 22 * 60,
            mode: ThermostatMode::Off,
            target_temp_f: 68.0,
        },
    ]
}
