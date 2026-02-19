use crate::{
    config::{PersistedSettings, ThermostatConfig},
    types::{ControllerStatePayload, ControllerStatus, ThermostatMode, ThermostatState},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldReason {
    ManualOverride,
    ExternalRemote,
    UserRequested,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EngineAction {
    PowerOn,
    PowerOff,
    HeatOn,
    HeatOff,
    TempUp,
    TempDown,
    SetTemp(i32),
    Delay(u64),
    LightToggle,
    TimerToggle,
}

#[derive(Debug, Clone, Copy)]
struct HoldState {
    start_ms: u64,
    duration_ms: u64,
    _reason: HoldReason,
}

#[derive(Debug, Clone)]
pub struct ThermostatEngine {
    pub config: ThermostatConfig,
    settings: PersistedSettings,

    state: ThermostatState,
    current_temp_f: f32,
    current_humidity: f32,
    fireplace_on: bool,

    last_sensor_update_ms: Option<u64>,
    last_state_change_ms: Option<u64>,

    hold: Option<HoldState>,

    heating_start_ms: Option<u64>,
    cooldown_start_ms: Option<u64>,
    in_cooldown: bool,

    previous_temp_f: Option<f32>,
    last_trend_sample_ms: Option<u64>,
    trend_direction: i8,
    consecutive_trend: u8,

    // Tracked fireplace device state used by IR action mapping.
    light_level: u8,
    timer_state: u8,
    fireplace_temp_f: i32,
}

impl ThermostatEngine {
    pub fn new(config: ThermostatConfig, mut settings: PersistedSettings) -> Self {
        settings.sanitize();
        Self {
            config,
            settings,
            state: ThermostatState::Idle,
            current_temp_f: 0.0,
            current_humidity: 0.0,
            fireplace_on: false,
            last_sensor_update_ms: None,
            last_state_change_ms: None,
            hold: None,
            heating_start_ms: None,
            cooldown_start_ms: None,
            in_cooldown: false,
            previous_temp_f: None,
            last_trend_sample_ms: None,
            trend_direction: 0,
            consecutive_trend: 0,
            light_level: 0,
            timer_state: 0,
            fireplace_temp_f: 70,
        }
    }

    pub fn settings(&self) -> &PersistedSettings {
        &self.settings
    }

    pub fn current_temp_f(&self) -> f32 {
        self.current_temp_f
    }

    pub fn current_humidity(&self) -> f32 {
        self.current_humidity
    }

    pub fn state(&self) -> ThermostatState {
        self.state
    }

    pub fn is_fireplace_on(&self) -> bool {
        self.fireplace_on
    }

    pub fn update_sensor_data(&mut self, temp_f: f32, humidity: f32, now_ms: u64) {
        self.current_temp_f = temp_f;
        self.current_humidity = humidity;
        self.last_sensor_update_ms = Some(now_ms);
    }

    pub fn set_target_temp(&mut self, temp_f: f32) -> bool {
        let clamped = temp_f.clamp(60.0, 84.0);
        if (self.settings.target_temp_f - clamped).abs() > f32::EPSILON {
            self.settings.target_temp_f = clamped;
            true
        } else {
            false
        }
    }

    pub fn set_hysteresis(&mut self, hysteresis_f: f32) -> bool {
        let clamped = hysteresis_f.clamp(0.5, 5.0);
        if (self.settings.hysteresis_f - clamped).abs() > f32::EPSILON {
            self.settings.hysteresis_f = clamped;
            true
        } else {
            false
        }
    }

    pub fn set_mode(&mut self, mode: ThermostatMode) -> bool {
        if self.settings.mode != mode {
            self.settings.mode = mode;
            if mode == ThermostatMode::Off {
                self.hold = None;
            }
            true
        } else {
            false
        }
    }

    pub fn set_mode_with_actions(
        &mut self,
        mode: ThermostatMode,
        now_ms: u64,
    ) -> (bool, Vec<EngineAction>) {
        let mut actions = Vec::new();
        let changed = self.set_mode(mode);

        if changed && mode == ThermostatMode::Off && self.fireplace_on {
            self.turn_fireplace_off(now_ms, &mut actions);
            self.state = ThermostatState::Idle;
        }

        (changed, actions)
    }

    pub fn set_fireplace_offset(&mut self, offset: i32) -> bool {
        if !(2..=10).contains(&offset) || offset % 2 != 0 {
            return false;
        }
        if self.settings.fireplace_offset_f != offset {
            self.settings.fireplace_offset_f = offset;
            true
        } else {
            false
        }
    }

    pub fn tick(&mut self, now_ms: u64) -> Vec<EngineAction> {
        let mut actions = Vec::new();

        self.expire_hold_if_needed(now_ms);
        self.complete_cooldown_if_needed(now_ms);
        self.check_runtime_limit(now_ms, &mut actions);
        self.detect_external_remote(now_ms);
        self.evaluate_state(now_ms, &mut actions);

        actions
    }

    pub fn manual_on(&mut self, now_ms: u64) -> Vec<EngineAction> {
        self.fireplace_on = true;
        self.heating_start_ms = Some(now_ms);
        self.last_state_change_ms = Some(now_ms);
        self.enter_hold_internal(
            self.config.hold_duration_ms,
            HoldReason::ManualOverride,
            now_ms,
        );
        vec![EngineAction::PowerOn]
    }

    pub fn manual_off(&mut self, now_ms: u64) -> Vec<EngineAction> {
        self.fireplace_on = false;
        self.heating_start_ms = None;
        self.last_state_change_ms = Some(now_ms);
        self.enter_hold_internal(
            self.config.hold_duration_ms,
            HoldReason::ManualOverride,
            now_ms,
        );
        vec![EngineAction::PowerOff]
    }

    pub fn manual_heat_on(&mut self, now_ms: u64) -> Vec<EngineAction> {
        self.enter_hold_internal(
            self.config.hold_duration_ms,
            HoldReason::ManualOverride,
            now_ms,
        );
        vec![EngineAction::HeatOn]
    }

    pub fn manual_heat_off(&mut self, now_ms: u64) -> Vec<EngineAction> {
        self.enter_hold_internal(
            self.config.hold_duration_ms,
            HoldReason::ManualOverride,
            now_ms,
        );
        vec![EngineAction::HeatOff]
    }

    pub fn manual_heat_up(&mut self) -> Vec<EngineAction> {
        if self.fireplace_temp_f >= 80 {
            return Vec::new();
        }
        self.fireplace_temp_f += 2;
        vec![EngineAction::TempUp]
    }

    pub fn manual_heat_down(&mut self) -> Vec<EngineAction> {
        if self.fireplace_temp_f <= 60 {
            return Vec::new();
        }
        self.fireplace_temp_f -= 2;
        vec![EngineAction::TempDown]
    }

    pub fn manual_light_toggle(&mut self) -> Vec<EngineAction> {
        self.advance_light_state();
        vec![EngineAction::LightToggle]
    }

    pub fn manual_timer_toggle(&mut self) -> Vec<EngineAction> {
        self.timer_state = (self.timer_state + 1) % 11;
        vec![EngineAction::TimerToggle]
    }

    pub fn enter_hold(&mut self, duration_ms: Option<u64>, now_ms: u64) {
        self.enter_hold_internal(
            duration_ms.unwrap_or(self.config.hold_duration_ms),
            HoldReason::UserRequested,
            now_ms,
        );
    }

    pub fn exit_hold(&mut self) {
        self.hold = None;
    }

    pub fn reset_safety(&mut self) {
        self.in_cooldown = false;
        self.cooldown_start_ms = None;
        self.heating_start_ms = None;
    }

    pub fn is_sensor_data_valid(&self, now_ms: u64) -> bool {
        self.last_sensor_update_ms
            .map(|last| now_ms.saturating_sub(last) < self.config.sensor_stale_timeout_ms)
            .unwrap_or(false)
    }

    pub fn last_sensor_update_ms(&self) -> Option<u64> {
        self.last_sensor_update_ms
    }

    pub fn is_in_hold(&self) -> bool {
        self.hold.is_some()
    }

    pub fn hold_remaining_ms(&self, now_ms: u64) -> u64 {
        match self.hold {
            Some(hold) => {
                let elapsed = now_ms.saturating_sub(hold.start_ms);
                hold.duration_ms.saturating_sub(elapsed)
            }
            None => 0,
        }
    }

    pub fn is_in_cooldown(&self) -> bool {
        self.in_cooldown
    }

    pub fn cooldown_remaining_ms(&self, now_ms: u64) -> u64 {
        if !self.in_cooldown {
            return 0;
        }
        let Some(start) = self.cooldown_start_ms else {
            return 0;
        };
        let elapsed = now_ms.saturating_sub(start);
        self.config.cooldown_duration_ms.saturating_sub(elapsed)
    }

    pub fn runtime_ms(&self, now_ms: u64) -> u64 {
        match self.heating_start_ms {
            Some(start) if self.fireplace_on => now_ms.saturating_sub(start),
            _ => 0,
        }
    }

    pub fn timer_string(&self) -> String {
        match self.timer_state {
            0 => "OFF".to_string(),
            1 => "0.5hr".to_string(),
            n => format!("{}hr", n - 1),
        }
    }

    pub fn apply_schedule_action(
        &mut self,
        mode: ThermostatMode,
        target_temp_f: f32,
        now_ms: u64,
    ) -> (bool, Vec<EngineAction>) {
        if self.is_in_hold() {
            return (false, Vec::new());
        }

        let mut actions = Vec::new();
        let mut changed = false;
        let (mode_changed, mut mode_actions) = self.set_mode_with_actions(mode, now_ms);
        changed |= mode_changed;
        actions.append(&mut mode_actions);
        changed |= self.set_target_temp(target_temp_f);

        (changed, actions)
    }

    pub fn status(
        &self,
        now_ms: u64,
        schedule_enabled: bool,
        next_schedule_event_epoch: Option<i64>,
        time_synced: bool,
        timezone: &str,
    ) -> ControllerStatus {
        ControllerStatus {
            current_temp: self.current_temp_f,
            current_humidity: self.current_humidity,
            target_temp: self.settings.target_temp_f,
            hysteresis: self.settings.hysteresis_f,
            fireplace_offset: self.settings.fireplace_offset_f,
            fireplace_temp: self.fireplace_temp_f,
            mode: self.settings.mode.as_str(),
            state: self.state.as_str(),
            fireplace_on: self.fireplace_on,
            sensor_valid: self.is_sensor_data_valid(now_ms),
            light_level: self.light_level,
            timer_state: self.timer_state,
            timer_string: self.timer_string(),
            hold_active: self.is_in_hold(),
            hold_remaining_ms: self.hold_remaining_ms(now_ms),
            hold_remaining_min: self.hold_remaining_ms(now_ms) / 60_000,
            in_cooldown: self.is_in_cooldown(),
            cooldown_remaining_ms: self.cooldown_remaining_ms(now_ms),
            cooldown_remaining_min: self.cooldown_remaining_ms(now_ms) / 60_000,
            runtime_ms: self.runtime_ms(now_ms),
            runtime_min: self.runtime_ms(now_ms) / 60_000,
            schedule_enabled,
            next_schedule_event_epoch,
            time_synced,
            timezone: timezone.to_string(),
        }
    }

    pub fn state_payload(&self, now_ms: u64) -> ControllerStatePayload {
        ControllerStatePayload {
            temp: self.current_temp_f,
            humidity: self.current_humidity,
            target: self.settings.target_temp_f,
            mode: self.settings.mode.as_str(),
            state: self.state.as_str(),
            fireplace: self.fireplace_on,
            hold_active: self.is_in_hold(),
            hold_remaining_min: self.hold_remaining_ms(now_ms) / 60_000,
            in_cooldown: self.is_in_cooldown(),
            cooldown_remaining_min: self.cooldown_remaining_ms(now_ms) / 60_000,
            runtime_min: self.runtime_ms(now_ms) / 60_000,
        }
    }

    fn enter_hold_internal(&mut self, duration_ms: u64, reason: HoldReason, now_ms: u64) {
        self.hold = Some(HoldState {
            start_ms: now_ms,
            duration_ms,
            _reason: reason,
        });
    }

    fn expire_hold_if_needed(&mut self, now_ms: u64) {
        if let Some(hold) = self.hold {
            if now_ms.saturating_sub(hold.start_ms) >= hold.duration_ms {
                self.hold = None;
            }
        }
    }

    fn complete_cooldown_if_needed(&mut self, now_ms: u64) {
        if self.cooldown_remaining_ms(now_ms) == 0 {
            self.in_cooldown = false;
            self.cooldown_start_ms = None;
        }
    }

    fn check_runtime_limit(&mut self, now_ms: u64, actions: &mut Vec<EngineAction>) {
        if !self.fireplace_on {
            return;
        }

        if self.runtime_ms(now_ms) >= self.config.max_runtime_ms && self.heating_start_ms.is_some()
        {
            actions.push(EngineAction::PowerOff);
            self.fireplace_on = false;
            self.in_cooldown = true;
            self.cooldown_start_ms = Some(now_ms);
            self.heating_start_ms = None;
            self.state = ThermostatState::Cooldown;
        }
    }

    fn detect_external_remote(&mut self, now_ms: u64) {
        if !self.is_sensor_data_valid(now_ms) {
            return;
        }

        if let Some(last) = self.last_trend_sample_ms {
            if now_ms.saturating_sub(last) < self.config.trend_sample_interval_ms {
                return;
            }
        }
        self.last_trend_sample_ms = Some(now_ms);

        let Some(previous) = self.previous_temp_f else {
            self.previous_temp_f = Some(self.current_temp_f);
            return;
        };

        let delta = self.current_temp_f - previous;
        self.previous_temp_f = Some(self.current_temp_f);

        let new_direction = if delta > self.config.trend_rising_threshold_f {
            1
        } else if delta < self.config.trend_falling_threshold_f {
            -1
        } else {
            0
        };

        if new_direction == self.trend_direction && new_direction != 0 {
            self.consecutive_trend = self.consecutive_trend.saturating_add(1);
        } else {
            self.consecutive_trend = if new_direction == 0 { 0 } else { 1 };
            self.trend_direction = new_direction;
        }

        if self.consecutive_trend < self.config.trend_samples_required {
            return;
        }

        if self.trend_direction == 1 && !self.fireplace_on {
            self.fireplace_on = true;
            self.heating_start_ms = Some(now_ms);
            self.enter_hold_internal(
                self.config.hold_duration_ms,
                HoldReason::ExternalRemote,
                now_ms,
            );
            self.consecutive_trend = 0;
        } else if self.trend_direction == -1 && self.fireplace_on {
            self.fireplace_on = false;
            self.heating_start_ms = None;
            self.enter_hold_internal(
                self.config.hold_duration_ms,
                HoldReason::ExternalRemote,
                now_ms,
            );
            self.consecutive_trend = 0;
        }
    }

    fn evaluate_state(&mut self, now_ms: u64, actions: &mut Vec<EngineAction>) {
        // Emergency shutoff: absolute max temperature ceiling
        if self.current_temp_f >= self.config.absolute_max_temp_f && self.fireplace_on {
            self.turn_fireplace_off(now_ms, actions);
            self.state = ThermostatState::Idle;
            return;
        }

        if self.settings.mode == ThermostatMode::Off {
            if self.fireplace_on {
                self.turn_fireplace_off(now_ms, actions);
            }
            self.state = ThermostatState::Idle;
            return;
        }

        if self.in_cooldown {
            self.state = ThermostatState::Cooldown;
            return;
        }

        if self.is_in_hold() {
            self.state = ThermostatState::Hold;
            return;
        }

        if !self.is_sensor_data_valid(now_ms) {
            if self.fireplace_on {
                self.turn_fireplace_off(now_ms, actions);
            }
            self.state = ThermostatState::Idle;
            return;
        }

        let lower_bound = self.settings.target_temp_f - self.settings.hysteresis_f;
        let upper_bound = self.settings.target_temp_f + self.settings.hysteresis_f;

        if !self.fireplace_on {
            if self.current_temp_f < lower_bound {
                if self.can_change_state(now_ms) {
                    self.turn_fireplace_on(now_ms, actions);
                }
            } else {
                self.state = ThermostatState::Satisfied;
            }
        } else if self.current_temp_f > upper_bound {
            if self.can_change_state(now_ms) {
                self.turn_fireplace_off(now_ms, actions);
            }
        } else {
            self.state = ThermostatState::Heating;
        }
    }

    fn can_change_state(&self, now_ms: u64) -> bool {
        self.last_state_change_ms
            .map(|last| now_ms.saturating_sub(last) >= self.config.min_cycle_ms)
            .unwrap_or(true)
    }

    fn turn_fireplace_on(&mut self, now_ms: u64, actions: &mut Vec<EngineAction>) {
        if self.fireplace_on {
            return;
        }

        actions.push(EngineAction::PowerOn);
        actions.push(EngineAction::Delay(500));
        actions.push(EngineAction::HeatOn);
        actions.push(EngineAction::Delay(200));

        let desired = Self::normalize_fireplace_temp(
            self.settings.target_temp_f as i32 + self.settings.fireplace_offset_f,
        );
        self.fireplace_temp_f = desired;
        actions.push(EngineAction::SetTemp(desired));
        actions.push(EngineAction::Delay(200));

        // Fireplace defaults light to 4 on power-on; send 4 toggles to return to OFF.
        self.light_level = 4;
        for step in 0..4 {
            actions.push(EngineAction::LightToggle);
            self.advance_light_state();
            if step < 3 {
                actions.push(EngineAction::Delay(200));
            }
        }

        self.fireplace_on = true;
        self.heating_start_ms = Some(now_ms);
        self.last_state_change_ms = Some(now_ms);
        self.state = ThermostatState::Heating;
    }

    fn turn_fireplace_off(&mut self, now_ms: u64, actions: &mut Vec<EngineAction>) {
        if !self.fireplace_on {
            return;
        }

        actions.push(EngineAction::PowerOff);
        self.fireplace_on = false;
        self.heating_start_ms = None;
        self.last_state_change_ms = Some(now_ms);
        self.state = ThermostatState::Satisfied;
    }

    fn normalize_fireplace_temp(temp: i32) -> i32 {
        let mut normalized = temp.clamp(60, 80);
        if normalized % 2 != 0 {
            normalized += 1;
        }
        normalized
    }

    fn advance_light_state(&mut self) {
        self.light_level = if self.light_level == 0 {
            4
        } else {
            self.light_level - 1
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_on_when_below_threshold() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());
        let mut settings = engine.settings.clone();
        settings.mode = ThermostatMode::Heat;
        settings.target_temp_f = 70.0;
        settings.hysteresis_f = 2.0;
        engine.settings = settings;

        engine.update_sensor_data(65.0, 40.0, 1_000);
        let actions = engine.tick(300_999);

        assert!(actions.contains(&EngineAction::PowerOn));
        assert!(engine.is_fireplace_on());
    }

    #[test]
    fn runtime_limit_triggers_cooldown() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());
        let mut settings = engine.settings.clone();
        settings.mode = ThermostatMode::Heat;
        engine.settings = settings;
        engine.fireplace_on = true;
        engine.heating_start_ms = Some(0);

        let actions = engine.tick(14_400_001);

        assert!(actions.contains(&EngineAction::PowerOff));
        assert!(!engine.is_fireplace_on());
        assert!(engine.is_in_cooldown());
        assert_eq!(engine.state(), ThermostatState::Cooldown);
    }

    #[test]
    fn hold_expires() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());
        engine.enter_hold(Some(1_000), 100);

        let _ = engine.tick(900);
        assert!(engine.is_in_hold());

        let _ = engine.tick(1_101);
        assert!(!engine.is_in_hold());
    }

    #[test]
    fn manual_heat_controls_respect_bounds() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());

        engine.fireplace_temp_f = 80;
        assert!(engine.manual_heat_up().is_empty());
        assert_eq!(engine.fireplace_temp_f, 80);

        engine.fireplace_temp_f = 60;
        assert!(engine.manual_heat_down().is_empty());
        assert_eq!(engine.fireplace_temp_f, 60);

        engine.fireplace_temp_f = 78;
        assert_eq!(engine.manual_heat_up(), vec![EngineAction::TempUp]);
        assert_eq!(engine.fireplace_temp_f, 80);

        engine.fireplace_temp_f = 62;
        assert_eq!(engine.manual_heat_down(), vec![EngineAction::TempDown]);
        assert_eq!(engine.fireplace_temp_f, 60);
    }

    #[test]
    fn manual_light_toggle_cycles_levels() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());
        let mut observed = Vec::new();

        for _ in 0..5 {
            assert_eq!(
                engine.manual_light_toggle(),
                vec![EngineAction::LightToggle]
            );
            observed.push(engine.light_level);
        }

        assert_eq!(observed, vec![4, 3, 2, 1, 0]);
    }

    #[test]
    fn manual_timer_toggle_cycles_states() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());

        for state in 1..=10 {
            assert_eq!(
                engine.manual_timer_toggle(),
                vec![EngineAction::TimerToggle]
            );
            assert_eq!(engine.timer_state, state);
        }

        assert_eq!(
            engine.manual_timer_toggle(),
            vec![EngineAction::TimerToggle]
        );
        assert_eq!(engine.timer_state, 0);
    }

    #[test]
    fn mode_off_immediately_turns_fireplace_off() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());
        engine.settings.mode = ThermostatMode::Heat;
        engine.fireplace_on = true;
        engine.heating_start_ms = Some(1_000);
        engine.last_state_change_ms = Some(1_200);

        let (changed, actions) = engine.set_mode_with_actions(ThermostatMode::Off, 1_300);

        assert!(changed);
        assert_eq!(actions, vec![EngineAction::PowerOff]);
        assert!(!engine.is_fireplace_on());
        assert_eq!(engine.state(), ThermostatState::Idle);
    }

    #[test]
    fn odd_target_rounds_up_to_even() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());
        let mut settings = engine.settings.clone();
        settings.mode = ThermostatMode::Heat;
        settings.target_temp_f = 69.0;
        settings.fireplace_offset_f = 4;
        engine.settings = settings;

        engine.update_sensor_data(60.0, 40.0, 0);
        let actions = engine.tick(299_999);

        assert!(actions.contains(&EngineAction::SetTemp(74)));
    }

    #[test]
    fn power_on_sequence_contains_required_delays() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());
        let mut settings = engine.settings.clone();
        settings.mode = ThermostatMode::Heat;
        settings.target_temp_f = 70.0;
        settings.hysteresis_f = 2.0;
        engine.settings = settings;

        engine.update_sensor_data(65.0, 40.0, 1_000);
        let actions = engine.tick(300_999);

        assert_eq!(actions.first(), Some(&EngineAction::PowerOn));
        assert_eq!(actions.get(1), Some(&EngineAction::Delay(500)));
        assert_eq!(actions.get(2), Some(&EngineAction::HeatOn));
        assert_eq!(actions.get(3), Some(&EngineAction::Delay(200)));
    }

    #[test]
    fn sensor_stale_bypasses_min_cycle() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());
        let mut settings = engine.settings.clone();
        settings.mode = ThermostatMode::Heat;
        engine.settings = settings;

        engine.fireplace_on = true;
        engine.heating_start_ms = Some(100);
        engine.last_state_change_ms = Some(100);
        engine.update_sensor_data(70.0, 40.0, 100);

        // Sensor goes stale (300s timeout exceeded)
        let actions = engine.tick(300_101);

        assert!(actions.contains(&EngineAction::PowerOff));
        assert!(!engine.is_fireplace_on());
        assert_eq!(engine.state(), ThermostatState::Idle);
    }

    #[test]
    fn absolute_max_temp_emergency_shutoff() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());
        let mut settings = engine.settings.clone();
        settings.mode = ThermostatMode::Heat;
        settings.target_temp_f = 70.0;
        engine.settings = settings;

        engine.fireplace_on = true;
        engine.heating_start_ms = Some(100);
        engine.update_sensor_data(95.0, 40.0, 500);

        let actions = engine.tick(600);

        assert!(actions.contains(&EngineAction::PowerOff));
        assert!(!engine.is_fireplace_on());
        assert_eq!(engine.state(), ThermostatState::Idle);
    }

    #[test]
    fn mode_off_bypasses_min_cycle() {
        let mut engine =
            ThermostatEngine::new(ThermostatConfig::default(), PersistedSettings::default());
        let mut settings = engine.settings.clone();
        settings.mode = ThermostatMode::Heat;
        engine.settings = settings;

        engine.fireplace_on = true;
        engine.heating_start_ms = Some(100);
        engine.last_state_change_ms = Some(100);
        engine.update_sensor_data(70.0, 40.0, 100);

        // Set mode to Off immediately (within min_cycle window)
        let (changed, actions) = engine.set_mode_with_actions(ThermostatMode::Off, 200);

        assert!(changed);
        assert!(actions.contains(&EngineAction::PowerOff));
        assert!(!engine.is_fireplace_on());
        assert_eq!(engine.state(), ThermostatState::Idle);
    }
}
