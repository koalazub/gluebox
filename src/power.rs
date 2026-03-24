use std::sync::Mutex;
use std::time::Instant;
use tokio::sync::watch;

use crate::config::PowerConfig;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PowerState {
    Active,
    Resting,
}

pub struct PowerManager {
    potential: Mutex<f64>,
    config: Mutex<PowerConfig>,
    state_tx: watch::Sender<PowerState>,
    state_rx: watch::Receiver<PowerState>,
    last_spike_at: Mutex<Instant>,
    active_since: Mutex<Option<Instant>>,
}

impl PowerManager {
    pub fn new(config: PowerConfig) -> anyhow::Result<Self> {
        Self::validate(&config)?;
        let (state_tx, state_rx) = watch::channel(PowerState::Resting);
        Ok(Self {
            potential: Mutex::new(0.0),
            config: Mutex::new(config),
            state_tx,
            state_rx,
            last_spike_at: Mutex::new(Instant::now()),
            active_since: Mutex::new(None),
        })
    }

    fn validate(config: &PowerConfig) -> anyhow::Result<()> {
        anyhow::ensure!(config.spike_weight > 0.0, "spike_weight must be > 0");
        anyhow::ensure!(config.decay_rate > 0.0, "decay_rate must be > 0");
        anyhow::ensure!(config.threshold > 0.0, "threshold must be > 0");
        anyhow::ensure!(config.tick_interval_secs > 0, "tick_interval_secs must be > 0");
        Ok(())
    }

    pub fn spike(&self) {
        let config = self.config.lock().unwrap();
        let mut potential = self.potential.lock().unwrap();
        *potential += config.spike_weight;
        *self.last_spike_at.lock().unwrap() = Instant::now();

        if *potential >= config.threshold {
            if *self.state_tx.borrow() == PowerState::Resting {
                *self.active_since.lock().unwrap() = Some(Instant::now());
                let _ = self.state_tx.send(PowerState::Active);
            }
        }
    }

    pub fn tick(&self) {
        let config = self.config.lock().unwrap();
        let mut potential = self.potential.lock().unwrap();
        *potential = (*potential - config.decay_rate).max(0.0);

        if *potential < config.threshold && *self.state_tx.borrow() == PowerState::Active {
            let active_since = self.active_since.lock().unwrap();
            if let Some(since) = *active_since {
                if since.elapsed().as_secs() >= config.min_active_secs {
                    drop(active_since);
                    *self.active_since.lock().unwrap() = None;
                    let _ = self.state_tx.send(PowerState::Resting);
                }
            }
        }
    }

    pub fn state(&self) -> PowerState {
        *self.state_tx.borrow()
    }

    pub fn potential(&self) -> f64 {
        *self.potential.lock().unwrap()
    }

    pub fn subscribe(&self) -> watch::Receiver<PowerState> {
        self.state_rx.clone()
    }

    pub fn reconfigure(&self, config: PowerConfig) -> anyhow::Result<()> {
        Self::validate(&config)?;
        *self.config.lock().unwrap() = config;
        Ok(())
    }

    pub fn tick_interval(&self) -> std::time::Duration {
        let secs = self.config.lock().unwrap().tick_interval_secs;
        std::time::Duration::from_secs(secs)
    }

    pub fn threshold(&self) -> f64 {
        self.config.lock().unwrap().threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(threshold: f64, decay_rate: f64, spike_weight: f64, min_active_secs: u64) -> PowerConfig {
        PowerConfig {
            threshold,
            decay_rate,
            tick_interval_secs: 1,
            spike_weight,
            min_active_secs,
        }
    }

    #[test]
    fn spike_accumulates_potential() {
        let pm = PowerManager::new(config_with(10.0, 0.5, 2.0, 0)).unwrap();
        pm.spike();
        assert_eq!(pm.potential(), 2.0);
        pm.spike();
        assert_eq!(pm.potential(), 4.0);
    }

    #[test]
    fn tick_decays_potential_never_below_zero() {
        let pm = PowerManager::new(config_with(10.0, 0.5, 2.0, 0)).unwrap();
        pm.spike();
        assert_eq!(pm.potential(), 2.0);
        pm.tick();
        assert_eq!(pm.potential(), 1.5);
        pm.tick();
        pm.tick();
        pm.tick();
        assert_eq!(pm.potential(), 0.0);
    }

    #[test]
    fn crossing_threshold_transitions_to_active() {
        let pm = PowerManager::new(config_with(3.0, 0.5, 2.0, 0)).unwrap();
        assert_eq!(pm.state(), PowerState::Resting);
        pm.spike();
        assert_eq!(pm.state(), PowerState::Resting);
        pm.spike();
        assert_eq!(pm.state(), PowerState::Active);
    }

    #[test]
    fn decaying_below_threshold_after_min_active_secs_transitions_to_resting() {
        let pm = PowerManager::new(config_with(3.0, 0.5, 2.0, 0)).unwrap();
        pm.spike();
        pm.spike();
        assert_eq!(pm.state(), PowerState::Active);
        pm.tick();
        pm.tick();
        pm.tick();
        pm.tick();
        pm.tick();
        pm.tick();
        assert_eq!(pm.state(), PowerState::Resting);
    }

    #[test]
    fn hysteresis_prevents_resting_before_min_active_secs() {
        let pm = PowerManager::new(config_with(3.0, 0.5, 2.0, 9999)).unwrap();
        pm.spike();
        pm.spike();
        assert_eq!(pm.state(), PowerState::Active);
        pm.tick();
        pm.tick();
        pm.tick();
        pm.tick();
        pm.tick();
        pm.tick();
        assert_eq!(pm.state(), PowerState::Active);
    }

    #[test]
    fn validation_rejects_zero_spike_weight() {
        let cfg = PowerConfig {
            threshold: 1.0,
            decay_rate: 0.5,
            tick_interval_secs: 1,
            spike_weight: 0.0,
            min_active_secs: 0,
        };
        assert!(PowerManager::new(cfg).is_err());
    }

    #[test]
    fn validation_rejects_zero_decay_rate() {
        let cfg = PowerConfig {
            threshold: 1.0,
            decay_rate: 0.0,
            tick_interval_secs: 1,
            spike_weight: 1.0,
            min_active_secs: 0,
        };
        assert!(PowerManager::new(cfg).is_err());
    }

    #[test]
    fn validation_rejects_zero_threshold() {
        let cfg = PowerConfig {
            threshold: 0.0,
            decay_rate: 0.5,
            tick_interval_secs: 1,
            spike_weight: 1.0,
            min_active_secs: 0,
        };
        assert!(PowerManager::new(cfg).is_err());
    }

    #[test]
    fn validation_rejects_zero_tick_interval() {
        let cfg = PowerConfig {
            threshold: 1.0,
            decay_rate: 0.5,
            tick_interval_secs: 0,
            spike_weight: 1.0,
            min_active_secs: 0,
        };
        assert!(PowerManager::new(cfg).is_err());
    }

    #[test]
    fn reconfigure_updates_config() {
        let pm = PowerManager::new(config_with(3.0, 0.5, 2.0, 0)).unwrap();
        assert_eq!(pm.threshold(), 3.0);
        let new_cfg = config_with(10.0, 1.0, 3.0, 0);
        pm.reconfigure(new_cfg).unwrap();
        assert_eq!(pm.threshold(), 10.0);
    }

    #[test]
    fn reconfigure_rejects_invalid_config() {
        let pm = PowerManager::new(config_with(3.0, 0.5, 2.0, 0)).unwrap();
        let bad_cfg = PowerConfig {
            threshold: 0.0,
            decay_rate: 0.5,
            tick_interval_secs: 1,
            spike_weight: 1.0,
            min_active_secs: 0,
        };
        assert!(pm.reconfigure(bad_cfg).is_err());
        assert_eq!(pm.threshold(), 3.0);
    }
}
