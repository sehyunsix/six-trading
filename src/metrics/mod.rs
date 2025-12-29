use hdrhistogram::Histogram;
use std::sync::Mutex;
use std::time::Duration;

pub struct SystemMetrics {
    // Receive -> Signal (Strategy)
    pub strategy_latency: Mutex<Histogram<u64>>,
    // Signal -> Order Confirmation (Execution)
    pub execution_latency: Mutex<Histogram<u64>>,
}

impl SystemMetrics {
    pub fn new() -> Self {
        Self {
            strategy_latency: Mutex::new(Histogram::<u64>::new(3).unwrap()),
            execution_latency: Mutex::new(Histogram::<u64>::new(3).unwrap()),
        }
    }

    pub fn record_strategy_latency(&self, duration: Duration) {
        let micros = duration.as_micros() as u64;
        let mut hist = self.strategy_latency.lock().unwrap();
        let _ = hist.record(micros);
    }

    pub fn record_execution_latency(&self, duration: Duration) {
        let micros = duration.as_micros() as u64;
        let mut hist = self.execution_latency.lock().unwrap();
        let _ = hist.record(micros);
    }

    pub fn get_strategy_stats(&self) -> LatencyStats {
        let hist = self.strategy_latency.lock().unwrap();
        Self::stats_from_hist(&hist)
    }

    pub fn get_execution_stats(&self) -> LatencyStats {
        let hist = self.execution_latency.lock().unwrap();
        Self::stats_from_hist(&hist)
    }

    fn stats_from_hist(hist: &Histogram<u64>) -> LatencyStats {
        LatencyStats {
            min: hist.min(),
            mean: hist.mean(),
            p50: hist.value_at_quantile(0.5),
            p90: hist.value_at_quantile(0.9),
            p99: hist.value_at_quantile(0.99),
            max: hist.max(),
        }
    }
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct LatencyStats {
    pub min: u64,
    pub mean: f64,
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
    pub max: u64,
}

