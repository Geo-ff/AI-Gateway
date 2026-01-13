use crate::config::{BalanceStrategy, Provider};
use rand::Rng;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub struct LoadBalancerState {
    provider_counter: AtomicUsize,
    per_provider_key_counter: Mutex<HashMap<String, usize>>,
}

impl LoadBalancerState {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    fn next_provider_index(&self, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        self.provider_counter.fetch_add(1, Ordering::Relaxed) % len
    }

    fn next_key_index(&self, provider_name: &str, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        let mut map = self
            .per_provider_key_counter
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let counter = map.entry(provider_name.to_string()).or_insert(0);
        let idx = *counter % len;
        *counter = counter.wrapping_add(1);
        idx
    }
}

pub struct LoadBalancer {
    providers: Vec<Provider>,
    strategy: BalanceStrategy,
    state: Arc<LoadBalancerState>,
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
    #[allow(dead_code)]
    pub fn new(providers: Vec<Provider>, strategy: BalanceStrategy) -> Self {
        Self::with_state(providers, strategy, Arc::new(LoadBalancerState::default()))
    }

    pub fn with_state(
        providers: Vec<Provider>,
        strategy: BalanceStrategy,
        state: Arc<LoadBalancerState>,
    ) -> Self {
        Self {
            providers,
            strategy,
            state,
        }
    }

    pub fn select_provider(&self) -> Result<SelectedProvider, BalanceError> {
        if self.providers.is_empty() {
            return Err(BalanceError::NoProvidersAvailable);
        }

        let provider = match self.strategy {
            BalanceStrategy::FirstAvailable => &self.providers[0],
            BalanceStrategy::RoundRobin => {
                let index = self.state.next_provider_index(self.providers.len());
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
                let index = self
                    .state
                    .next_key_index(&provider.name, provider.api_keys.len());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BalanceStrategy, ProviderType};

    fn provider(name: &str, keys: &[&str]) -> Provider {
        Provider {
            name: name.to_string(),
            api_type: ProviderType::OpenAI,
            base_url: "http://example.invalid".to_string(),
            api_keys: keys.iter().map(|s| s.to_string()).collect(),
            models_endpoint: None,
        }
    }

    #[test]
    fn round_robin_persists_across_instances_and_rotates_keys_per_provider() {
        let providers = vec![provider("p0", &["k0a", "k0b"]), provider("p1", &["k1a", "k1b"])];
        let state = Arc::new(LoadBalancerState::default());

        let mut out: Vec<(String, String)> = Vec::new();
        for _ in 0..4 {
            let lb = LoadBalancer::with_state(providers.clone(), BalanceStrategy::RoundRobin, state.clone());
            let s = lb.select_provider().unwrap();
            out.push((s.provider.name, s.api_key));
        }

        assert_eq!(
            out,
            vec![
                ("p0".to_string(), "k0a".to_string()),
                ("p1".to_string(), "k1a".to_string()),
                ("p0".to_string(), "k0b".to_string()),
                ("p1".to_string(), "k1b".to_string()),
            ]
        );
    }
}
