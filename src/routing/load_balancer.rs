use crate::config::{BalanceStrategy, Provider};
use crate::routing::{KeyRotationStrategy, ProviderKeyEntry};
use rand::Rng;
use rand::distr::{Distribution, weighted::WeightedIndex};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub struct LoadBalancerState {
    provider_counter: AtomicUsize,
    per_provider_key_counter: Mutex<HashMap<String, usize>>,
    per_provider_swrr_state: Mutex<HashMap<String, HashMap<String, i64>>>,
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

    fn next_weighted_sequential_index(
        &self,
        provider_name: &str,
        active_keys: &[&ProviderKeyEntry],
    ) -> usize {
        if active_keys.is_empty() {
            return 0;
        }
        let mut state_map = self
            .per_provider_swrr_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let state = state_map
            .entry(provider_name.to_string())
            .or_insert_with(HashMap::new);

        let active_set: std::collections::HashSet<&str> =
            active_keys.iter().map(|e| e.value.as_str()).collect();
        state.retain(|k, _| active_set.contains(k.as_str()));

        let mut total_weight: i64 = 0;
        let mut best_idx: usize = 0;
        let mut best_val: i64 = i64::MIN;
        for (idx, entry) in active_keys.iter().enumerate() {
            let w = i64::from(entry.weight.max(1));
            total_weight += w;
            let cur = state.entry(entry.value.clone()).or_insert(0);
            *cur += w;
            if *cur > best_val {
                best_val = *cur;
                best_idx = idx;
            }
        }
        if total_weight <= 0 {
            return 0;
        }
        let selected_key = active_keys[best_idx].value.clone();
        if let Some(v) = state.get_mut(&selected_key) {
            *v -= total_weight;
        }
        best_idx
    }

    pub fn select_provider_key(
        &self,
        provider_name: &str,
        strategy: KeyRotationStrategy,
        keys: &[ProviderKeyEntry],
    ) -> Result<String, BalanceError> {
        let mut rng = rand::rng();
        self.select_provider_key_with_rng(provider_name, strategy, keys, &mut rng)
    }

    pub fn select_provider_key_with_rng<R: Rng + ?Sized>(
        &self,
        provider_name: &str,
        strategy: KeyRotationStrategy,
        keys: &[ProviderKeyEntry],
        rng: &mut R,
    ) -> Result<String, BalanceError> {
        let active: Vec<&ProviderKeyEntry> = keys
            .iter()
            .filter(|e| e.active && !e.value.is_empty() && e.weight >= 1)
            .collect();
        if active.is_empty() {
            return Err(BalanceError::NoApiKeysAvailable);
        }

        let idx = match strategy {
            KeyRotationStrategy::Sequential => self.next_key_index(provider_name, active.len()),
            KeyRotationStrategy::Random => rng.random_range(0..active.len()),
            KeyRotationStrategy::WeightedRandom => {
                let weights: Vec<u32> = active.iter().map(|e| e.weight.max(1)).collect();
                let dist =
                    WeightedIndex::new(&weights).map_err(|_| BalanceError::NoApiKeysAvailable)?;
                dist.sample(rng)
            }
            KeyRotationStrategy::WeightedSequential => {
                self.next_weighted_sequential_index(provider_name, &active)
            }
        };

        Ok(active[idx].value.clone())
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

    pub fn select_provider_only(&self) -> Result<Provider, BalanceError> {
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
        Ok(provider.clone())
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
    use crate::config::settings::DEFAULT_PROVIDER_COLLECTION;
    use crate::config::{BalanceStrategy, ProviderType};
    use rand::SeedableRng;

    fn provider(name: &str, keys: &[&str]) -> Provider {
        Provider {
            name: name.to_string(),
            display_name: None,
            collection: DEFAULT_PROVIDER_COLLECTION.to_string(),
            api_type: ProviderType::OpenAI,
            base_url: "http://example.invalid".to_string(),
            api_keys: keys.iter().map(|s| s.to_string()).collect(),
            models_endpoint: None,
            enabled: true,
        }
    }

    #[test]
    fn round_robin_persists_across_instances_and_rotates_keys_per_provider() {
        let providers = vec![
            provider("p0", &["k0a", "k0b"]),
            provider("p1", &["k1a", "k1b"]),
        ];
        let state = Arc::new(LoadBalancerState::default());

        let mut out: Vec<(String, String)> = Vec::new();
        for _ in 0..4 {
            let lb = LoadBalancer::with_state(
                providers.clone(),
                BalanceStrategy::RoundRobin,
                state.clone(),
            );
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

    #[test]
    fn weighted_random_is_reasonable_and_disabled_keys_not_selected() {
        let state = LoadBalancerState::default();
        let keys = vec![
            ProviderKeyEntry {
                value: "a".into(),
                active: true,
                weight: 1,
            },
            ProviderKeyEntry {
                value: "b".into(),
                active: true,
                weight: 3,
            },
            ProviderKeyEntry {
                value: "c".into(),
                active: false,
                weight: 100,
            },
        ];

        // disabled-only => error
        let disabled_only = vec![ProviderKeyEntry {
            value: "x".into(),
            active: false,
            weight: 1,
        }];
        assert!(matches!(
            state.select_provider_key("p0", KeyRotationStrategy::Random, &disabled_only),
            Err(BalanceError::NoApiKeysAvailable)
        ));

        // deterministic RNG to avoid flaky tests
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        let mut a = 0usize;
        let mut b = 0usize;
        for _ in 0..10_000 {
            let picked = state
                .select_provider_key_with_rng(
                    "p0",
                    KeyRotationStrategy::WeightedRandom,
                    &keys,
                    &mut rng,
                )
                .unwrap();
            match picked.as_str() {
                "a" => a += 1,
                "b" => b += 1,
                // disabled "c" should never appear
                _ => panic!("unexpected key: {}", picked),
            }
        }
        let ratio = b as f64 / a as f64;
        assert!(ratio > 2.5 && ratio < 3.5, "ratio={}", ratio);
    }

    #[test]
    fn weighted_sequential_uses_smooth_weighted_round_robin() {
        let state = LoadBalancerState::default();
        let keys = vec![
            ProviderKeyEntry {
                value: "a".into(),
                active: true,
                weight: 1,
            },
            ProviderKeyEntry {
                value: "b".into(),
                active: true,
                weight: 2,
            },
        ];
        let mut out = Vec::new();
        for _ in 0..6 {
            out.push(
                state
                    .select_provider_key("p0", KeyRotationStrategy::WeightedSequential, &keys)
                    .unwrap(),
            );
        }
        assert_eq!(out, vec!["b", "a", "b", "b", "a", "b"]);
    }
}
