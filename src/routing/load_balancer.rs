use crate::config::{BalanceStrategy, Provider};
use rand::Rng;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct LoadBalancer {
    providers: Vec<Provider>,
    strategy: BalanceStrategy,
    round_robin_counter: AtomicUsize,
}

#[derive(Debug)]
pub enum BalanceError {
    NoProvidersAvailable,
    NoApiKeysAvailable,
}

impl std::fmt::Display for BalanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BalanceError::NoProvidersAvailable => write!(f, "No providers available"),
            BalanceError::NoApiKeysAvailable => write!(f, "No API keys available"),
        }
    }
}

impl std::error::Error for BalanceError {}

pub struct SelectedProvider {
    pub provider: Provider,
    pub api_key: String,
}

impl LoadBalancer {
    pub fn new(providers: Vec<Provider>, strategy: BalanceStrategy) -> Self {
        Self {
            providers,
            strategy,
            round_robin_counter: AtomicUsize::new(0),
        }
    }

    pub fn select_provider(&self) -> Result<SelectedProvider, BalanceError> {
        if self.providers.is_empty() {
            return Err(BalanceError::NoProvidersAvailable);
        }

        let provider = match self.strategy {
            BalanceStrategy::FirstAvailable => &self.providers[0],
            BalanceStrategy::RoundRobin => {
                let index = self.round_robin_counter.fetch_add(1, Ordering::Relaxed)
                    % self.providers.len();
                &self.providers[index]
            }
            BalanceStrategy::Random => {
                let mut rng = rand::rng();
                let index = rng.random_range(0..self.providers.len());
                &self.providers[index]
            }
        };

        let api_key = self.select_api_key(provider)?;

        Ok(SelectedProvider {
            provider: provider.clone(),
            api_key,
        })
    }

    fn select_api_key(&self, provider: &Provider) -> Result<String, BalanceError> {
        if provider.api_keys.is_empty() {
            return Err(BalanceError::NoApiKeysAvailable);
        }

        match self.strategy {
            BalanceStrategy::FirstAvailable => Ok(provider.api_keys[0].clone()),
            BalanceStrategy::RoundRobin => {
                let index = self.round_robin_counter.load(Ordering::Relaxed)
                    % provider.api_keys.len();
                Ok(provider.api_keys[index].clone())
            }
            BalanceStrategy::Random => {
                let mut rng = rand::rng();
                let index = rng.random_range(0..provider.api_keys.len());
                Ok(provider.api_keys[index].clone())
            }
        }
    }
}