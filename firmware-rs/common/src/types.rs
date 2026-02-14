use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ThermostatMode {
    Off,
    Heat,
}

impl ThermostatMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Heat => "HEAT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ThermostatState {
    Idle,
    Heating,
    Satisfied,
    Hold,
    Cooldown,
}

impl ThermostatState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Heating => "HEATING",
            Self::Satisfied => "SATISFIED",
            Self::Hold => "HOLD",
            Self::Cooldown => "COOLDOWN",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ControllerStatus {
    #[serde(rename = "currentTemp")]
    pub current_temp: f32,
    #[serde(rename = "currentHumidity")]
    pub current_humidity: f32,
    #[serde(rename = "targetTemp")]
    pub target_temp: f32,
    pub hysteresis: f32,
    #[serde(rename = "fireplaceOffset")]
    pub fireplace_offset: i32,
    #[serde(rename = "fireplaceTemp")]
    pub fireplace_temp: i32,
    pub mode: &'static str,
    pub state: &'static str,
    #[serde(rename = "fireplaceOn")]
    pub fireplace_on: bool,
    #[serde(rename = "sensorValid")]
    pub sensor_valid: bool,
    #[serde(rename = "lightLevel")]
    pub light_level: u8,
    #[serde(rename = "timerState")]
    pub timer_state: u8,
    #[serde(rename = "timerString")]
    pub timer_string: String,
    #[serde(rename = "holdActive")]
    pub hold_active: bool,
    #[serde(rename = "holdRemainingMs")]
    pub hold_remaining_ms: u64,
    #[serde(rename = "holdRemainingMin")]
    pub hold_remaining_min: u64,
    #[serde(rename = "inCooldown")]
    pub in_cooldown: bool,
    #[serde(rename = "cooldownRemainingMs")]
    pub cooldown_remaining_ms: u64,
    #[serde(rename = "cooldownRemainingMin")]
    pub cooldown_remaining_min: u64,
    #[serde(rename = "runtimeMs")]
    pub runtime_ms: u64,
    #[serde(rename = "runtimeMin")]
    pub runtime_min: u64,
    #[serde(rename = "scheduleEnabled")]
    pub schedule_enabled: bool,
    #[serde(rename = "nextScheduleEventEpoch")]
    pub next_schedule_event_epoch: Option<i64>,
    #[serde(rename = "timeSynced")]
    pub time_synced: bool,
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ControllerStatePayload {
    pub temp: f32,
    pub humidity: f32,
    pub target: f32,
    pub mode: &'static str,
    pub state: &'static str,
    pub fireplace: bool,
    #[serde(rename = "holdActive")]
    pub hold_active: bool,
    #[serde(rename = "holdRemainingMin")]
    pub hold_remaining_min: u64,
    #[serde(rename = "inCooldown")]
    pub in_cooldown: bool,
    #[serde(rename = "cooldownRemainingMin")]
    pub cooldown_remaining_min: u64,
    #[serde(rename = "runtimeMin")]
    pub runtime_min: u64,
}
