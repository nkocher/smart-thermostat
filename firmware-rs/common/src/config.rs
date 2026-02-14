use serde::{Deserialize, Serialize};

use crate::types::ThermostatMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThermostatConfig {
    pub min_cycle_ms: u64,
    pub sensor_stale_timeout_ms: u64,
    pub state_publish_interval_ms: u64,
    pub trend_sample_interval_ms: u64,
    pub trend_rising_threshold_f: f32,
    pub trend_falling_threshold_f: f32,
    pub trend_samples_required: u8,
    pub max_runtime_ms: u64,
    pub cooldown_duration_ms: u64,
    pub hold_duration_ms: u64,
    pub settings_save_debounce_ms: u64,
    pub min_valid_temp_f: f32,
    pub max_valid_temp_f: f32,
    pub max_hold_minutes: u16,
    pub absolute_max_temp_f: f32,
}

impl Default for ThermostatConfig {
    fn default() -> Self {
        Self {
            min_cycle_ms: 300_000,
            sensor_stale_timeout_ms: 300_000,
            state_publish_interval_ms: 10_000,
            trend_sample_interval_ms: 30_000,
            trend_rising_threshold_f: 0.3,
            trend_falling_threshold_f: -0.2,
            trend_samples_required: 3,
            max_runtime_ms: 14_400_000,
            cooldown_duration_ms: 1_800_000,
            hold_duration_ms: 1_800_000,
            settings_save_debounce_ms: 5_000,
            min_valid_temp_f: -40.0,
            max_valid_temp_f: 150.0,
            max_hold_minutes: 1_440,
            absolute_max_temp_f: 95.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSettings {
    pub target_temp_f: f32,
    pub hysteresis_f: f32,
    pub mode: ThermostatMode,
    pub fireplace_offset_f: i32,
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            target_temp_f: 70.0,
            hysteresis_f: 2.0,
            mode: ThermostatMode::Off,
            fireplace_offset_f: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub wifi_ssid: String,
    pub wifi_pass: String,
    pub mqtt_host: String,
    pub mqtt_port: u16,
    pub mqtt_user: String,
    pub mqtt_pass: String,
    pub ota_password: String,
    pub use_static_ip: bool,
    pub static_ip: Option<[u8; 4]>,
    pub gateway: Option<[u8; 4]>,
    pub subnet: Option<[u8; 4]>,
    pub dns: Option<[u8; 4]>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            wifi_ssid: String::new(),
            wifi_pass: String::new(),
            mqtt_host: "192.168.1.100".to_string(),
            mqtt_port: 1883,
            mqtt_user: String::new(),
            mqtt_pass: String::new(),
            ota_password: String::new(),
            use_static_ip: false,
            static_ip: None,
            gateway: None,
            subnet: None,
            dns: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IrHardwareConfig {
    pub tx_pin: i32,
    pub rmt_channel: u8,
    pub carrier_khz: u32,
}

impl Default for IrHardwareConfig {
    fn default() -> Self {
        Self {
            tx_pin: 4,
            rmt_channel: 0,
            carrier_khz: 36,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub thermostat: ThermostatConfig,
    pub settings: PersistedSettings,
    pub timezone: String,
    pub network: NetworkConfig,
    #[serde(default)]
    pub ir: IrHardwareConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            thermostat: ThermostatConfig::default(),
            settings: PersistedSettings::default(),
            timezone: "America/Los_Angeles".to_string(),
            network: NetworkConfig::default(),
            ir: IrHardwareConfig::default(),
        }
    }
}

impl PersistedSettings {
    pub fn sanitize(&mut self) {
        self.target_temp_f = self.target_temp_f.clamp(60.0, 84.0);
        self.hysteresis_f = self.hysteresis_f.clamp(0.5, 5.0);

        let clamped = self.fireplace_offset_f.clamp(2, 10);
        self.fireplace_offset_f = if clamped % 2 == 0 {
            clamped
        } else {
            clamped - 1
        };
        if self.fireplace_offset_f < 2 {
            self.fireplace_offset_f = 2;
        }
    }
}

impl IrHardwareConfig {
    pub fn sanitize(&mut self) {
        if self.tx_pin < 0 {
            self.tx_pin = 4;
        }

        if self.rmt_channel > 7 {
            self.rmt_channel = 0;
        }

        self.carrier_khz = self.carrier_khz.clamp(10, 100);
    }
}
