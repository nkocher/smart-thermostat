use core::convert::TryInto;
use std::{
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context};
use esp_idf_hal::{
    gpio::OutputPin,
    peripheral::Peripheral,
    rmt::{
        config::{CarrierConfig, DutyPercent, TransmitConfig},
        PinState, Pulse, PulseTicks, RmtChannel, TxRmtDriver, VariableLengthSignal,
    },
    units::FromValueType,
};
use log::{info, warn};
use serde::Serialize;

use thermostat_common::EngineAction;

use crate::ir_codes;

const IR_TICK_DIVIDER: u8 = 80;
const IR_CARRIER_FREQ_KHZ: u32 = 36;
const IR_REPEAT_COUNT: usize = 3;
const IR_REPEAT_GAP_MS: u64 = 50;
const MIN_SEND_INTERVAL_MS: u64 = 300;

#[derive(Debug, Clone)]
struct IrRuntimeState {
    current_temp_f: i32,
    light_level: u8,
    timer_state: u8,
}

impl Default for IrRuntimeState {
    fn default() -> Self {
        Self {
            current_temp_f: 70,
            light_level: 0,
            timer_state: 0,
        }
    }
}

enum IrBackend {
    Rmt(TxRmtDriver<'static>),
    Disabled,
}

pub struct IrTransmitter {
    backend: IrBackend,
    state: IrRuntimeState,
    last_send_ms: Option<u64>,
    carrier_khz: u32,
    sent_frames: u64,
    failed_actions: u64,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IrDiagnostics {
    pub enabled: bool,
    #[serde(rename = "carrierKHz")]
    pub carrier_khz: u32,
    #[serde(rename = "repeatCount")]
    pub repeat_count: usize,
    #[serde(rename = "repeatGapMs")]
    pub repeat_gap_ms: u64,
    #[serde(rename = "minSendIntervalMs")]
    pub min_send_interval_ms: u64,
    #[serde(rename = "lastSendMs")]
    pub last_send_ms: Option<u64>,
    #[serde(rename = "sentFrames")]
    pub sent_frames: u64,
    #[serde(rename = "failedActions")]
    pub failed_actions: u64,
    #[serde(rename = "lastError")]
    pub last_error: Option<String>,
    #[serde(rename = "runtimeTempF")]
    pub runtime_temp_f: i32,
    #[serde(rename = "runtimeLightLevel")]
    pub runtime_light_level: u8,
    #[serde(rename = "runtimeTimerState")]
    pub runtime_timer_state: u8,
}

impl IrTransmitter {
    pub fn new<C, P>(
        channel: impl Peripheral<P = C> + 'static,
        pin: impl Peripheral<P = P> + 'static,
    ) -> anyhow::Result<Self>
    where
        C: RmtChannel,
        P: OutputPin,
    {
        Self::new_with_carrier(channel, pin, IR_CARRIER_FREQ_KHZ)
    }

    pub fn new_with_carrier<C, P>(
        channel: impl Peripheral<P = C> + 'static,
        pin: impl Peripheral<P = P> + 'static,
        carrier_khz: u32,
    ) -> anyhow::Result<Self>
    where
        C: RmtChannel,
        P: OutputPin,
    {
        let carrier = CarrierConfig::new()
            .frequency(carrier_khz.kHz().into())
            .carrier_level(PinState::High)
            .duty_percent(DutyPercent::new(33)?);

        let config = TransmitConfig::new()
            .clock_divider(IR_TICK_DIVIDER)
            .carrier(Some(carrier))
            .idle(Some(PinState::Low));

        let tx = TxRmtDriver::new(channel, pin, &config).context("failed to init RMT IR driver")?;

        Ok(Self {
            backend: IrBackend::Rmt(tx),
            state: IrRuntimeState::default(),
            last_send_ms: None,
            carrier_khz,
            sent_frames: 0,
            failed_actions: 0,
            last_error: None,
        })
    }

    pub fn disabled() -> Self {
        Self {
            backend: IrBackend::Disabled,
            state: IrRuntimeState::default(),
            last_send_ms: None,
            carrier_khz: IR_CARRIER_FREQ_KHZ,
            sent_frames: 0,
            failed_actions: 0,
            last_error: None,
        }
    }

    pub fn execute_action(&mut self, action: EngineAction) -> anyhow::Result<()> {
        let result = (|| -> anyhow::Result<()> {
            match action {
                EngineAction::PowerOn => {
                    self.send_raw(ir_codes::IR_RAW_POWER_ON)?;
                    self.state.light_level = 4;
                }
                EngineAction::PowerOff => {
                    self.send_raw(ir_codes::IR_RAW_POWER_OFF)?;
                }
                EngineAction::HeatOn => {
                    self.send_raw(ir_codes::IR_RAW_HEAT_ON)?;
                }
                EngineAction::HeatOff => {
                    self.send_raw(ir_codes::IR_RAW_HEAT_OFF)?;
                }
                EngineAction::TempUp => {
                    if !self.send_temp_up_transition()? {
                        info!("ignoring TEMP UP at max temperature");
                    }
                }
                EngineAction::TempDown => {
                    if !self.send_temp_down_transition()? {
                        info!("ignoring TEMP DOWN at min temperature");
                    }
                }
                EngineAction::SetTemp(target) => {
                    self.set_temp(target)?;
                }
                EngineAction::Delay(_) => {}
                EngineAction::LightToggle => {
                    self.send_light_toggle()?;
                }
                EngineAction::TimerToggle => {
                    self.send_timer_toggle()?;
                }
            }
            Ok(())
        })();

        if let Err(err) = &result {
            self.failed_actions = self.failed_actions.saturating_add(1);
            self.last_error = Some(format!("{err:#}"));
        } else {
            self.last_error = None;
        }

        result
    }

    pub fn diagnostics(&self) -> IrDiagnostics {
        IrDiagnostics {
            enabled: matches!(self.backend, IrBackend::Rmt(_)),
            carrier_khz: self.carrier_khz,
            repeat_count: IR_REPEAT_COUNT,
            repeat_gap_ms: IR_REPEAT_GAP_MS,
            min_send_interval_ms: MIN_SEND_INTERVAL_MS,
            last_send_ms: self.last_send_ms,
            sent_frames: self.sent_frames,
            failed_actions: self.failed_actions,
            last_error: self.last_error.clone(),
            runtime_temp_f: self.state.current_temp_f,
            runtime_light_level: self.state.light_level,
            runtime_timer_state: self.state.timer_state,
        }
    }

    fn set_temp(&mut self, target_temp_f: i32) -> anyhow::Result<()> {
        let target = normalize_temp(target_temp_f);

        while self.state.current_temp_f < target {
            if !self.send_temp_up_transition()? {
                break;
            }
        }

        while self.state.current_temp_f > target {
            if !self.send_temp_down_transition()? {
                break;
            }
        }

        Ok(())
    }

    fn send_temp_up_transition(&mut self) -> anyhow::Result<bool> {
        if self.state.current_temp_f >= 80 {
            return Ok(false);
        }

        let code = temp_up_code(self.state.current_temp_f)
            .ok_or_else(|| anyhow!("missing temp-up code for {}", self.state.current_temp_f))?;
        self.send_raw(code)?;
        self.state.current_temp_f += 2;
        Ok(true)
    }

    fn send_temp_down_transition(&mut self) -> anyhow::Result<bool> {
        if self.state.current_temp_f <= 60 {
            return Ok(false);
        }

        let code = temp_down_code(self.state.current_temp_f)
            .ok_or_else(|| anyhow!("missing temp-down code for {}", self.state.current_temp_f))?;
        self.send_raw(code)?;
        self.state.current_temp_f -= 2;
        Ok(true)
    }

    fn send_light_toggle(&mut self) -> anyhow::Result<()> {
        let code = match self.state.light_level {
            0 => ir_codes::IR_RAW_LIGHT_FROM_OFF,
            4 => ir_codes::IR_RAW_LIGHT_FROM_4,
            3 => ir_codes::IR_RAW_LIGHT_FROM_3,
            2 => ir_codes::IR_RAW_LIGHT_FROM_2,
            1 => ir_codes::IR_RAW_LIGHT_FROM_1,
            level => return Err(anyhow!("invalid light level state: {level}")),
        };

        self.send_raw(code)?;
        self.state.light_level = if self.state.light_level == 0 {
            4
        } else {
            self.state.light_level - 1
        };

        Ok(())
    }

    fn send_timer_toggle(&mut self) -> anyhow::Result<()> {
        let code = match self.state.timer_state {
            0 => ir_codes::IR_RAW_TIMER_FROM_OFF,
            1 => ir_codes::IR_RAW_TIMER_FROM_0_5,
            2 => ir_codes::IR_RAW_TIMER_FROM_1,
            3 => ir_codes::IR_RAW_TIMER_FROM_2,
            4 => ir_codes::IR_RAW_TIMER_FROM_3,
            5 => ir_codes::IR_RAW_TIMER_FROM_4,
            6 => ir_codes::IR_RAW_TIMER_FROM_5,
            7 => ir_codes::IR_RAW_TIMER_FROM_6,
            8 => ir_codes::IR_RAW_TIMER_FROM_7,
            9 => ir_codes::IR_RAW_TIMER_FROM_8,
            10 => ir_codes::IR_RAW_TIMER_FROM_9,
            state => return Err(anyhow!("invalid timer state: {state}")),
        };

        self.send_raw(code)?;
        self.state.timer_state = (self.state.timer_state + 1) % 11;

        Ok(())
    }

    fn send_raw(&mut self, raw: &[u16]) -> anyhow::Result<()> {
        if raw.is_empty() {
            return Ok(());
        }

        if matches!(self.backend, IrBackend::Disabled) {
            warn!("IR disabled, dropping frame with {} timings", raw.len());
            return Ok(());
        }

        self.rate_limit();

        let mut pulses = Vec::with_capacity(raw.len());
        for (index, duration) in raw.iter().enumerate() {
            let level = if index % 2 == 0 {
                PinState::High
            } else {
                PinState::Low
            };

            pulses.push(Pulse::new(
                level,
                PulseTicks::new(*duration).context("invalid IR pulse duration")?,
            ));
        }

        let pulse_refs: Vec<&Pulse> = pulses.iter().collect();
        let mut signal = VariableLengthSignal::with_capacity(pulses.len());
        signal
            .push(pulse_refs)
            .context("failed to convert IR timings to RMT signal")?;

        if let IrBackend::Rmt(tx) = &mut self.backend {
            for repeat in 0..IR_REPEAT_COUNT {
                tx.start_blocking(&signal)
                    .context("failed to transmit IR frame over RMT")?;
                if repeat + 1 < IR_REPEAT_COUNT {
                    thread::sleep(Duration::from_millis(IR_REPEAT_GAP_MS));
                }
            }
        }

        self.last_send_ms = Some(monotonic_ms());
        self.sent_frames = self.sent_frames.saturating_add(1);
        Ok(())
    }

    fn rate_limit(&mut self) {
        let now = monotonic_ms();
        if let Some(last) = self.last_send_ms {
            let elapsed = now.saturating_sub(last);
            if elapsed < MIN_SEND_INTERVAL_MS {
                thread::sleep(Duration::from_millis(MIN_SEND_INTERVAL_MS - elapsed));
            }
        }
    }
}

fn temp_up_code(current_temp_f: i32) -> Option<&'static [u16]> {
    match current_temp_f {
        60 => Some(ir_codes::IR_RAW_TEMP_UP_FROM_60),
        62 => Some(ir_codes::IR_RAW_TEMP_UP_FROM_62),
        64 => Some(ir_codes::IR_RAW_TEMP_UP_FROM_64),
        66 => Some(ir_codes::IR_RAW_TEMP_UP_FROM_66),
        68 => Some(ir_codes::IR_RAW_TEMP_UP_FROM_68),
        70 => Some(ir_codes::IR_RAW_TEMP_UP_FROM_70),
        72 => Some(ir_codes::IR_RAW_TEMP_UP_FROM_72),
        74 => Some(ir_codes::IR_RAW_TEMP_UP_FROM_74),
        76 => Some(ir_codes::IR_RAW_TEMP_UP_FROM_76),
        78 => Some(ir_codes::IR_RAW_TEMP_UP_FROM_78),
        _ => None,
    }
}

fn temp_down_code(current_temp_f: i32) -> Option<&'static [u16]> {
    match current_temp_f {
        80 => Some(ir_codes::IR_RAW_TEMP_DOWN_FROM_80),
        78 => Some(ir_codes::IR_RAW_TEMP_DOWN_FROM_78),
        76 => Some(ir_codes::IR_RAW_TEMP_DOWN_FROM_76),
        74 => Some(ir_codes::IR_RAW_TEMP_DOWN_FROM_74),
        72 => Some(ir_codes::IR_RAW_TEMP_DOWN_FROM_72),
        70 => Some(ir_codes::IR_RAW_TEMP_DOWN_FROM_70),
        68 => Some(ir_codes::IR_RAW_TEMP_DOWN_FROM_68),
        66 => Some(ir_codes::IR_RAW_TEMP_DOWN_FROM_66),
        64 => Some(ir_codes::IR_RAW_TEMP_DOWN_FROM_64),
        62 => Some(ir_codes::IR_RAW_TEMP_DOWN_FROM_62),
        _ => None,
    }
}

fn normalize_temp(temp_f: i32) -> i32 {
    let mut normalized = temp_f.clamp(60, 80);
    if normalized % 2 != 0 {
        normalized += 1;
    }
    normalized
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
