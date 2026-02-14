use std::{
    collections::HashMap,
    io::ErrorKind,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};

use anyhow::Context;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use chrono::{Offset, Utc};
use chrono_tz::Tz;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, sync::Mutex};
use tower_http::services::ServeDir;
use tracing::{info, warn};

use thermostat_common::{
    config::{IrHardwareConfig, NetworkConfig},
    DayOfWeek, EngineAction, RuntimeConfig, Schedule, ScheduleAction, ScheduleEntry,
    ThermostatEngine, ThermostatMode, TOPIC_CMD_HOLD, TOPIC_CMD_MODE, TOPIC_CMD_POWER,
    TOPIC_CMD_SCHEDULE, TOPIC_CMD_TARGET, TOPIC_CONTROLLER_SCHEDULE_STATE, TOPIC_CONTROLLER_STATE,
    TOPIC_SENSOR_HUMIDITY, TOPIC_SENSOR_TEMP,
};

#[derive(Clone)]
struct AppState {
    engine: Arc<Mutex<ThermostatEngine>>,
    schedule: Arc<Mutex<Schedule>>,
    timezone: Arc<Mutex<String>>,
    time_synced: Arc<AtomicBool>,
    mqtt: AsyncClient,
    store: AppStore,
}

#[derive(Clone)]
struct AppStore {
    runtime_path: Arc<PathBuf>,
    schedule_path: Arc<PathBuf>,
    lock: Arc<Mutex<()>>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Deserialize)]
struct TimezoneUpdate {
    timezone: String,
}

#[derive(Debug, Serialize)]
struct TimeStatus {
    #[serde(rename = "timeSynced")]
    time_synced: bool,
    timezone: String,
    #[serde(rename = "nowEpoch")]
    now_epoch: i64,
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

const MAX_MQTT_PAYLOAD_BYTES: usize = 512;

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

#[derive(Debug, Serialize)]
struct IrDiagnosticsView {
    enabled: bool,
    #[serde(rename = "txPin")]
    tx_pin: i32,
    #[serde(rename = "rmtChannel")]
    rmt_channel: u8,
    #[serde(rename = "carrierKHz")]
    carrier_khz: u32,
    #[serde(rename = "sentFrames")]
    sent_frames: u64,
    #[serde(rename = "failedActions")]
    failed_actions: u64,
    #[serde(rename = "lastError")]
    last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OtaApplyRequest {
    url: String,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Serialize)]
struct OtaStatusResponse {
    supported: bool,
    #[serde(rename = "inProgress")]
    in_progress: bool,
    #[serde(rename = "lastError")]
    last_error: Option<String>,
}

pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let store = AppStore::new();
    let mut runtime = store.load_runtime_config().await.unwrap_or_else(|err| {
        warn!("failed to load runtime config from store: {err:#}");
        RuntimeConfig::default()
    });
    runtime.settings.sanitize();

    let mut schedule = store.load_schedule().await.unwrap_or_else(|err| {
        warn!("failed to load schedule from store: {err:#}");
        Schedule::default()
    });
    schedule.normalize();

    let engine = ThermostatEngine::new(runtime.thermostat.clone(), runtime.settings.clone());

    let mqtt_host = std::env::var("MQTT_HOST").unwrap_or(runtime.network.mqtt_host.clone());
    let mqtt_port = std::env::var("MQTT_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(runtime.network.mqtt_port);

    let mut mqtt_options = MqttOptions::new("thermostat-controller-rust", mqtt_host, mqtt_port);
    let mqtt_user = std::env::var("MQTT_USER").unwrap_or(runtime.network.mqtt_user.clone());
    let mqtt_pass = std::env::var("MQTT_PASS").unwrap_or(runtime.network.mqtt_pass.clone());
    if !mqtt_user.is_empty() {
        mqtt_options.set_credentials(mqtt_user, mqtt_pass);
    }

    let (mqtt, eventloop) = AsyncClient::new(mqtt_options, 64);

    let app_state = AppState {
        engine: Arc::new(Mutex::new(engine)),
        schedule: Arc::new(Mutex::new(schedule)),
        timezone: Arc::new(Mutex::new(runtime.timezone)),
        time_synced: Arc::new(AtomicBool::new(false)),
        mqtt,
        store,
    };

    subscribe_topics(&app_state.mqtt).await?;
    spawn_mqtt_loop(app_state.clone(), eventloop);
    spawn_control_loop(app_state.clone());
    spawn_state_publish_loop(app_state.clone());

    let web_root = format!("{}/web", env!("CARGO_MANIFEST_DIR"));
    let app = Router::new()
        .route("/api/status", get(handle_get_status))
        .route("/api/target", post(handle_set_target))
        .route("/api/mode", post(handle_set_mode))
        .route("/api/hysteresis", post(handle_set_hysteresis))
        .route("/api/offset", post(handle_set_offset))
        .route("/api/ir/on", post(handle_ir_on))
        .route("/api/ir/off", post(handle_ir_off))
        .route("/api/ir/heat/on", post(handle_ir_heat_on))
        .route("/api/ir/heat/off", post(handle_ir_heat_off))
        .route("/api/ir/heat/up", post(handle_ir_heat_up))
        .route("/api/ir/heat/down", post(handle_ir_heat_down))
        .route("/api/ir/light/toggle", post(handle_ir_light_toggle))
        .route("/api/ir/timer/toggle", post(handle_ir_timer_toggle))
        .route(
            "/api/ir/config",
            get(handle_get_ir_config).put(handle_put_ir_config),
        )
        .route("/api/ir/diagnostics", get(handle_get_ir_diagnostics))
        .route("/api/hold/enter", post(handle_hold_enter))
        .route("/api/hold/exit", post(handle_hold_exit))
        .route("/api/safety/reset", post(handle_safety_reset))
        .route(
            "/api/schedule",
            get(handle_get_schedule).put(handle_put_schedule),
        )
        .route("/api/time", get(handle_get_time))
        .route("/api/timezone", put(handle_put_timezone))
        .route(
            "/api/network",
            get(handle_get_network).put(handle_put_network),
        )
        .route("/api/ota/status", get(handle_get_ota_status))
        .route("/api/ota/apply", post(handle_post_ota_apply))
        .fallback_service(ServeDir::new(web_root))
        .with_state(app_state);

    let port = std::env::var("CONTROLLER_HTTP_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8080);
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind controller server at {addr}"))?;

    info!("controller listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn subscribe_topics(mqtt: &AsyncClient) -> anyhow::Result<()> {
    let topics = [
        TOPIC_SENSOR_TEMP,
        TOPIC_SENSOR_HUMIDITY,
        TOPIC_CMD_POWER,
        TOPIC_CMD_TARGET,
        TOPIC_CMD_MODE,
        TOPIC_CMD_HOLD,
        TOPIC_CMD_SCHEDULE,
    ];

    for topic in topics {
        mqtt.subscribe(topic, QoS::AtMostOnce).await?;
    }
    Ok(())
}

fn spawn_mqtt_loop(app_state: AppState, mut eventloop: rumqttc::EventLoop) {
    tokio::spawn(async move {
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Incoming::Publish(message))) => {
                    if let Err(err) =
                        handle_mqtt_message(&app_state, message.topic, message.payload.to_vec())
                            .await
                    {
                        warn!("mqtt message handling error: {err:#}");
                    }
                }
                Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                    info!("mqtt connected");
                }
                Ok(_) => {}
                Err(err) => {
                    warn!("mqtt poll error: {err}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    });
}

fn spawn_control_loop(app_state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));

        loop {
            interval.tick().await;
            let now_ms = monotonic_ms();

            let timezone = { app_state.timezone.lock().await.clone() };
            let now_in_tz = now_in_timezone(&timezone);
            app_state
                .time_synced
                .store(now_in_tz.is_some(), Ordering::Relaxed);

            if let Some(now) = now_in_tz {
                let schedule_action = {
                    let schedule = app_state.schedule.lock().await;
                    schedule.current_action(now)
                };

                if let Some(ScheduleAction {
                    mode,
                    target_temp_f,
                }) = schedule_action
                {
                    let schedule_actions = {
                        let mut engine = app_state.engine.lock().await;
                        let (_, schedule_actions) =
                            engine.apply_schedule_action(mode, target_temp_f, now_ms);
                        schedule_actions
                    };

                    if !schedule_actions.is_empty() {
                        execute_engine_actions(schedule_actions).await;
                    }
                }
            }

            let actions = {
                let mut engine = app_state.engine.lock().await;
                engine.tick(now_ms)
            };

            if !actions.is_empty() {
                execute_engine_actions(actions).await;
            }
        }
    });
}

fn spawn_state_publish_loop(app_state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;

            let now_ms = monotonic_ms();
            let payload = {
                let engine = app_state.engine.lock().await;
                serde_json::to_vec(&engine.state_payload(now_ms))
            };

            match payload {
                Ok(body) => {
                    if let Err(err) = app_state
                        .mqtt
                        .publish(TOPIC_CONTROLLER_STATE, QoS::AtLeastOnce, true, body)
                        .await
                    {
                        warn!("controller state publish failed: {err}");
                    }
                }
                Err(err) => warn!("controller state serialization failed: {err}"),
            }

            let schedule_payload = {
                let schedule = app_state.schedule.lock().await;
                serde_json::to_vec(&*schedule)
            };

            match schedule_payload {
                Ok(body) => {
                    if let Err(err) = app_state
                        .mqtt
                        .publish(
                            TOPIC_CONTROLLER_SCHEDULE_STATE,
                            QoS::AtLeastOnce,
                            true,
                            body,
                        )
                        .await
                    {
                        warn!("schedule state publish failed: {err}");
                    }
                }
                Err(err) => warn!("schedule serialization failed: {err}"),
            }
        }
    });
}

async fn execute_engine_actions(actions: Vec<EngineAction>) {
    for action in actions {
        if let EngineAction::Delay(ms) = action {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            continue;
        }

        // This preserves behavior sequencing in one place; ESP32 IR transport hooks in here.
        info!("engine action: {action:?}");
    }
}

async fn handle_mqtt_message(
    app_state: &AppState,
    topic: String,
    payload: Vec<u8>,
) -> anyhow::Result<()> {
    if payload.len() > MAX_MQTT_PAYLOAD_BYTES {
        warn!(
            "dropping oversized MQTT payload on topic {} ({} bytes)",
            topic,
            payload.len()
        );
        return Ok(());
    }

    let message = String::from_utf8(payload).context("non utf8 mqtt payload")?;
    let now_ms = monotonic_ms();

    match topic.as_str() {
        TOPIC_SENSOR_TEMP => {
            if let Ok(temp) = message.parse::<f32>() {
                if temp.is_finite() && (-40.0..=150.0).contains(&temp) {
                    let mut engine = app_state.engine.lock().await;
                    let humidity = engine.current_humidity();
                    engine.update_sensor_data(temp, humidity, now_ms);
                }
            }
        }
        TOPIC_SENSOR_HUMIDITY => {
            if let Ok(humidity) = message.parse::<f32>() {
                if humidity.is_finite() && (0.0..=100.0).contains(&humidity) {
                    let mut engine = app_state.engine.lock().await;
                    let temp = engine.current_temp_f();
                    engine.update_sensor_data(temp, humidity, now_ms);
                }
            }
        }
        TOPIC_CMD_POWER => {
            let lower = message.to_ascii_lowercase();
            let actions = {
                let mut engine = app_state.engine.lock().await;
                if lower == "on" {
                    engine.manual_on(now_ms)
                } else if lower == "off" {
                    engine.manual_off(now_ms)
                } else {
                    Vec::new()
                }
            };
            execute_engine_actions(actions).await;
        }
        TOPIC_CMD_TARGET => {
            if let Ok(target) = message.parse::<f32>() {
                let changed = {
                    let mut engine = app_state.engine.lock().await;
                    engine.set_target_temp(target)
                };
                if changed {
                    persist_runtime_from_state(app_state).await?;
                }
            }
        }
        TOPIC_CMD_MODE => {
            let upper = message.to_ascii_uppercase();
            let (changed, actions) = {
                let mut engine = app_state.engine.lock().await;
                if upper == "HEAT" {
                    engine.set_mode_with_actions(ThermostatMode::Heat, now_ms)
                } else if upper == "OFF" {
                    engine.set_mode_with_actions(ThermostatMode::Off, now_ms)
                } else {
                    (false, Vec::new())
                }
            };
            if !actions.is_empty() {
                execute_engine_actions(actions).await;
            }
            if changed {
                persist_runtime_from_state(app_state).await?;
            }
        }
        TOPIC_CMD_HOLD => {
            let lower = message.to_ascii_lowercase();
            let mut engine = app_state.engine.lock().await;
            if lower == "on" || lower == "enter" {
                engine.enter_hold(None, now_ms);
            } else if lower == "off" || lower == "exit" {
                engine.exit_hold();
            } else if let Ok(minutes) = lower.parse::<u64>() {
                if minutes > 0 && minutes <= engine.config.max_hold_minutes as u64 {
                    engine.enter_hold(Some(minutes * 60_000), now_ms);
                }
            }
        }
        TOPIC_CMD_SCHEDULE => {
            if let Ok(mut schedule) = serde_json::from_str::<Schedule>(&message) {
                schedule.normalize();
                {
                    let mut active = app_state.schedule.lock().await;
                    *active = schedule.clone();
                }
                app_state.store.save_schedule(&schedule).await?;
            }
        }
        _ => {}
    }

    Ok(())
}

async fn handle_get_status(State(state): State<AppState>) -> impl IntoResponse {
    let now_ms = monotonic_ms();
    let timezone = state.timezone.lock().await.clone();

    let next_schedule = {
        let schedule = state.schedule.lock().await;
        now_in_timezone(&timezone).and_then(|now| schedule.next_event_epoch(now))
    };

    let schedule_enabled = state.schedule.lock().await.enabled;
    let time_synced = state.time_synced.load(Ordering::Relaxed);

    let status = {
        let engine = state.engine.lock().await;
        engine.status(
            now_ms,
            schedule_enabled,
            next_schedule,
            time_synced,
            &timezone,
        )
    };

    Json(status)
}

async fn handle_set_target(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(value) = params.get("value") else {
        return error_response(StatusCode::BAD_REQUEST, "Missing 'value' parameter");
    };
    let Ok(target) = value.parse::<f32>() else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid temperature value");
    };

    let changed = {
        let mut engine = state.engine.lock().await;
        engine.set_target_temp(target)
    };

    if changed {
        if let Err(err) = persist_runtime_from_state(&state).await {
            warn!("failed to persist target update: {err:#}");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to persist runtime settings",
            );
        }
    }

    handle_get_status(State(state)).await.into_response()
}

async fn handle_set_mode(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(value) = params.get("value") else {
        return error_response(StatusCode::BAD_REQUEST, "Missing 'value' parameter");
    };

    let mode = match value.to_ascii_uppercase().as_str() {
        "HEAT" => ThermostatMode::Heat,
        "OFF" => ThermostatMode::Off,
        _ => return error_response(StatusCode::BAD_REQUEST, "Invalid mode. Use 'HEAT' or 'OFF'"),
    };

    let now_ms = monotonic_ms();
    let (changed, actions) = {
        let mut engine = state.engine.lock().await;
        engine.set_mode_with_actions(mode, now_ms)
    };
    if !actions.is_empty() {
        execute_engine_actions(actions).await;
    }

    if changed {
        if let Err(err) = persist_runtime_from_state(&state).await {
            warn!("failed to persist mode update: {err:#}");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to persist runtime settings",
            );
        }
    }

    handle_get_status(State(state)).await.into_response()
}

async fn handle_set_hysteresis(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(value) = params.get("value") else {
        return error_response(StatusCode::BAD_REQUEST, "Missing 'value' parameter");
    };
    let Ok(hysteresis) = value.parse::<f32>() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Invalid hysteresis value (0.5-5.0)",
        );
    };

    if !(0.5..=5.0).contains(&hysteresis) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Invalid hysteresis value (0.5-5.0)",
        );
    }

    let changed = {
        let mut engine = state.engine.lock().await;
        engine.set_hysteresis(hysteresis)
    };

    if changed {
        if let Err(err) = persist_runtime_from_state(&state).await {
            warn!("failed to persist hysteresis update: {err:#}");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to persist runtime settings",
            );
        }
    }

    handle_get_status(State(state)).await.into_response()
}

async fn handle_set_offset(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let Some(value) = params.get("value") else {
        return error_response(StatusCode::BAD_REQUEST, "Missing 'value' parameter");
    };
    let Ok(offset) = value.parse::<i32>() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Invalid offset value (2-10, even only)",
        );
    };

    if !(2..=10).contains(&offset) || offset % 2 != 0 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Invalid offset value (2-10, even only)",
        );
    }

    let changed = {
        let mut engine = state.engine.lock().await;
        engine.set_fireplace_offset(offset)
    };

    if changed {
        if let Err(err) = persist_runtime_from_state(&state).await {
            warn!("failed to persist offset update: {err:#}");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to persist runtime settings",
            );
        }
    }

    handle_get_status(State(state)).await.into_response()
}

async fn handle_ir_on(State(state): State<AppState>) -> impl IntoResponse {
    let actions = {
        let mut engine = state.engine.lock().await;
        engine.manual_on(monotonic_ms())
    };
    execute_engine_actions(actions).await;
    handle_get_status(State(state)).await.into_response()
}

async fn handle_ir_off(State(state): State<AppState>) -> impl IntoResponse {
    let actions = {
        let mut engine = state.engine.lock().await;
        engine.manual_off(monotonic_ms())
    };
    execute_engine_actions(actions).await;
    handle_get_status(State(state)).await.into_response()
}

async fn handle_ir_heat_on(State(state): State<AppState>) -> impl IntoResponse {
    let actions = {
        let mut engine = state.engine.lock().await;
        engine.manual_heat_on(monotonic_ms())
    };
    execute_engine_actions(actions).await;
    handle_get_status(State(state)).await.into_response()
}

async fn handle_ir_heat_off(State(state): State<AppState>) -> impl IntoResponse {
    let actions = {
        let mut engine = state.engine.lock().await;
        engine.manual_heat_off(monotonic_ms())
    };
    execute_engine_actions(actions).await;
    handle_get_status(State(state)).await.into_response()
}

async fn handle_ir_heat_up(State(state): State<AppState>) -> impl IntoResponse {
    let actions = {
        let mut engine = state.engine.lock().await;
        engine.manual_heat_up()
    };
    execute_engine_actions(actions).await;
    handle_get_status(State(state)).await.into_response()
}

async fn handle_ir_heat_down(State(state): State<AppState>) -> impl IntoResponse {
    let actions = {
        let mut engine = state.engine.lock().await;
        engine.manual_heat_down()
    };
    execute_engine_actions(actions).await;
    handle_get_status(State(state)).await.into_response()
}

async fn handle_ir_light_toggle(State(state): State<AppState>) -> impl IntoResponse {
    let actions = {
        let mut engine = state.engine.lock().await;
        engine.manual_light_toggle()
    };
    execute_engine_actions(actions).await;
    handle_get_status(State(state)).await.into_response()
}

async fn handle_ir_timer_toggle(State(state): State<AppState>) -> impl IntoResponse {
    let actions = {
        let mut engine = state.engine.lock().await;
        engine.manual_timer_toggle()
    };
    execute_engine_actions(actions).await;
    handle_get_status(State(state)).await.into_response()
}

async fn handle_hold_enter(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let duration_ms = params
        .get("minutes")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|minutes| *minutes > 0)
        .map(|minutes| minutes * 60_000);

    {
        let mut engine = state.engine.lock().await;
        engine.enter_hold(duration_ms, monotonic_ms());
    }

    handle_get_status(State(state)).await.into_response()
}

async fn handle_hold_exit(State(state): State<AppState>) -> impl IntoResponse {
    {
        let mut engine = state.engine.lock().await;
        engine.exit_hold();
    }
    handle_get_status(State(state)).await.into_response()
}

async fn handle_safety_reset(State(state): State<AppState>) -> impl IntoResponse {
    {
        let mut engine = state.engine.lock().await;
        engine.reset_safety();
    }
    handle_get_status(State(state)).await.into_response()
}

async fn handle_get_schedule(State(state): State<AppState>) -> impl IntoResponse {
    let schedule = state.schedule.lock().await.clone();
    Json(schedule)
}

async fn handle_put_schedule(
    State(state): State<AppState>,
    Json(mut schedule): Json<Schedule>,
) -> impl IntoResponse {
    schedule.normalize();
    {
        let mut active = state.schedule.lock().await;
        *active = schedule.clone();
    }

    if let Err(err) = state.store.save_schedule(&schedule).await {
        warn!("failed to persist schedule update: {err:#}");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to persist schedule",
        );
    }

    handle_get_schedule(State(state)).await.into_response()
}

async fn handle_get_time(State(state): State<AppState>) -> impl IntoResponse {
    let timezone = state.timezone.lock().await.clone();
    Json(TimeStatus {
        time_synced: state.time_synced.load(Ordering::Relaxed),
        timezone,
        now_epoch: Utc::now().timestamp(),
    })
}

async fn handle_put_timezone(
    State(state): State<AppState>,
    Json(update): Json<TimezoneUpdate>,
) -> impl IntoResponse {
    if update.timezone.parse::<Tz>().is_err() {
        return error_response(StatusCode::BAD_REQUEST, "Invalid timezone value");
    }

    {
        let mut timezone = state.timezone.lock().await;
        *timezone = update.timezone;
    }

    if let Err(err) = persist_runtime_from_state(&state).await {
        warn!("failed to persist timezone update: {err:#}");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to persist runtime settings",
        );
    }

    handle_get_time(State(state)).await.into_response()
}

async fn handle_get_network(State(state): State<AppState>) -> impl IntoResponse {
    let runtime = state
        .store
        .load_runtime_config()
        .await
        .unwrap_or_else(|err| {
            warn!("failed to load network config from store: {err:#}");
            RuntimeConfig::default()
        });
    Json(build_network_config_view(&runtime.network))
}

async fn handle_put_network(
    State(state): State<AppState>,
    Json(update): Json<NetworkConfigUpdate>,
) -> impl IntoResponse {
    if update.wifi_ssid.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "wifiSsid cannot be empty");
    }
    if update.mqtt_host.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "mqttHost cannot be empty");
    }
    if update.mqtt_port == 0 {
        return error_response(
            StatusCode::BAD_REQUEST,
            "mqttPort must be between 1 and 65535",
        );
    }
    if update.use_static_ip
        && (update.static_ip.is_none() || update.gateway.is_none() || update.subnet.is_none())
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            "staticIp, gateway, and subnet are required when useStaticIp is true",
        );
    }

    let mut runtime = state
        .store
        .load_runtime_config()
        .await
        .unwrap_or_else(|err| {
            warn!("failed to load existing runtime config for update: {err:#}");
            RuntimeConfig::default()
        });

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

    if let Err(err) = state.store.save_runtime_config(&runtime).await {
        warn!("failed to persist network config update: {err:#}");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to persist network settings",
        );
    }

    let payload = NetworkUpdateResponse {
        restart_required: network_restart_required(&previous, &runtime.network),
        network: build_network_config_view(&runtime.network),
    };
    Json(payload).into_response()
}

async fn handle_get_ir_config(State(state): State<AppState>) -> impl IntoResponse {
    let runtime = state
        .store
        .load_runtime_config()
        .await
        .unwrap_or_else(|err| {
            warn!("failed to load ir config from store: {err:#}");
            RuntimeConfig::default()
        });
    Json(build_ir_config_view(&runtime.ir))
}

async fn handle_put_ir_config(
    State(state): State<AppState>,
    Json(update): Json<IrConfigUpdate>,
) -> impl IntoResponse {
    if let Err(message) = validate_ir_update(&update) {
        return error_response(StatusCode::BAD_REQUEST, message);
    }

    let mut runtime = state
        .store
        .load_runtime_config()
        .await
        .unwrap_or_else(|err| {
            warn!("failed to load existing runtime config for ir update: {err:#}");
            RuntimeConfig::default()
        });

    let previous = runtime.ir.clone();
    runtime.ir.tx_pin = update.tx_pin;
    runtime.ir.rmt_channel = update.rmt_channel;
    runtime.ir.carrier_khz = update.carrier_khz;
    runtime.ir.sanitize();

    if let Err(err) = state.store.save_runtime_config(&runtime).await {
        warn!("failed to persist ir config update: {err:#}");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to persist ir settings",
        );
    }

    let payload = IrConfigUpdateResponse {
        restart_required: ir_restart_required(&previous, &runtime.ir),
        ir: build_ir_config_view(&runtime.ir),
    };
    Json(payload).into_response()
}

async fn handle_get_ir_diagnostics(State(state): State<AppState>) -> impl IntoResponse {
    let runtime = state
        .store
        .load_runtime_config()
        .await
        .unwrap_or_else(|err| {
            warn!("failed to load runtime config for ir diagnostics: {err:#}");
            RuntimeConfig::default()
        });

    let payload = IrDiagnosticsView {
        enabled: false,
        tx_pin: runtime.ir.tx_pin,
        rmt_channel: runtime.ir.rmt_channel,
        carrier_khz: runtime.ir.carrier_khz,
        sent_frames: 0,
        failed_actions: 0,
        last_error: Some("IR transmission is only available in ESP32 builds".to_string()),
    };
    Json(payload)
}

async fn handle_get_ota_status() -> impl IntoResponse {
    Json(OtaStatusResponse {
        supported: false,
        in_progress: false,
        last_error: Some("OTA apply is only available in ESP32 builds".to_string()),
    })
}

async fn handle_post_ota_apply(Json(request): Json<OtaApplyRequest>) -> impl IntoResponse {
    let _ = (request.url.as_str(), request.sha256.as_deref());
    error_response(
        StatusCode::NOT_IMPLEMENTED,
        "OTA apply is only available in ESP32 builds",
    )
}

impl AppStore {
    fn new() -> Self {
        let data_dir = std::env::var("THERMOSTAT_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./.thermostat"));

        Self {
            runtime_path: Arc::new(data_dir.join("runtime.json")),
            schedule_path: Arc::new(data_dir.join("schedule.json")),
            lock: Arc::new(Mutex::new(())),
        }
    }

    async fn load_runtime_config(&self) -> anyhow::Result<RuntimeConfig> {
        let _guard = self.lock.lock().await;
        match tokio::fs::read(self.runtime_path.as_ref()).await {
            Ok(raw) => Ok(serde_json::from_slice::<RuntimeConfig>(&raw)?),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(RuntimeConfig::default()),
            Err(err) => Err(err.into()),
        }
    }

    async fn save_runtime_config(&self, runtime: &RuntimeConfig) -> anyhow::Result<()> {
        let _guard = self.lock.lock().await;
        let path = self.runtime_path.as_ref().clone();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let payload = serde_json::to_vec_pretty(runtime)?;
        tokio::fs::write(path, payload).await?;
        Ok(())
    }

    async fn load_schedule(&self) -> anyhow::Result<Schedule> {
        let _guard = self.lock.lock().await;
        match tokio::fs::read(self.schedule_path.as_ref()).await {
            Ok(raw) => Ok(serde_json::from_slice::<Schedule>(&raw)?),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(Schedule::default()),
            Err(err) => Err(err.into()),
        }
    }

    async fn save_schedule(&self, schedule: &Schedule) -> anyhow::Result<()> {
        let _guard = self.lock.lock().await;
        let path = self.schedule_path.as_ref().clone();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let payload = serde_json::to_vec_pretty(schedule)?;
        tokio::fs::write(path, payload).await?;
        Ok(())
    }
}

async fn persist_runtime_from_state(state: &AppState) -> anyhow::Result<()> {
    let settings = state.engine.lock().await.settings().clone();
    let timezone = state.timezone.lock().await.clone();

    let mut runtime = state.store.load_runtime_config().await?;
    runtime.settings = settings;
    runtime.timezone = timezone;
    state.store.save_runtime_config(&runtime).await
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

fn build_ir_config_view(ir: &IrHardwareConfig) -> IrConfigView {
    IrConfigView {
        tx_pin: ir.tx_pin,
        rmt_channel: ir.rmt_channel,
        carrier_khz: ir.carrier_khz,
    }
}

fn validate_ir_update(update: &IrConfigUpdate) -> Result<(), &'static str> {
    if update.tx_pin < 0 {
        return Err("txPin must be >= 0");
    }
    if !is_supported_rmt_channel(update.rmt_channel) {
        return Err("rmtChannel is not supported");
    }
    if !(10..=100).contains(&update.carrier_khz) {
        return Err("carrierKHz must be between 10 and 100");
    }
    Ok(())
}

fn ir_restart_required(previous: &IrHardwareConfig, current: &IrHardwareConfig) -> bool {
    previous != current
}

fn is_supported_rmt_channel(channel: u8) -> bool {
    matches!(channel, 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7)
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

fn now_in_timezone(timezone: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    let tz: Tz = timezone.parse().ok()?;
    let local = Utc::now().with_timezone(&tz);
    Some(local.with_timezone(&local.offset().fix()))
}

fn error_response(status: StatusCode, message: &str) -> axum::response::Response {
    (
        status,
        Json(ErrorBody {
            error: message.to_string(),
        }),
    )
        .into_response()
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
