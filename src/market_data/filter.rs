use log::warn;
use binance::model::{TradeEvent, AggrTradesEvent};
use super::MarketEvent;

pub struct DataFilter {
    last_trade_id: u64,
    last_agg_trade_id: u64,
    last_timestamp: u64,
    last_price: Option<f64>,
    outlier_threshold: f64, 
    
    pub total_received: u64,
    pub duplicate_count: u64,
    pub out_of_order_count: u64,
    pub outlier_count: u64,
}

impl DataFilter {
    pub fn new(outlier_threshold: f64) -> Self {
        Self {
            last_trade_id: 0,
            last_agg_trade_id: 0,
            last_timestamp: 0,
            last_price: None,
            outlier_threshold,
            total_received: 0,
            duplicate_count: 0,
            out_of_order_count: 0,
            outlier_count: 0,
        }
    }

    pub fn should_process(&mut self, event: &MarketEvent) -> bool {
        self.total_received += 1;
        
        match event {
            MarketEvent::Trade(trade) => self.filter_trade(trade),
            MarketEvent::AggrTrade(agg) => self.filter_agg_trade(agg),
            _ => true, // OrderBook and others pass for now
        }
    }

    fn filter_trade(&mut self, trade: &TradeEvent) -> bool {
        // 1. Check Duplicates
        if trade.trade_id <= self.last_trade_id && self.last_trade_id != 0 {
            self.duplicate_count += 1;
            warn!("Filtered duplicate trade ID: {}", trade.trade_id);
            return false;
        }
        self.last_trade_id = trade.trade_id;

        // 2. Check Out-of-order
        if trade.event_time < self.last_timestamp && self.last_timestamp != 0 {
            self.out_of_order_count += 1;
            warn!("Filtered out-of-order trade: {} < {}", trade.event_time, self.last_timestamp);
            return false;
        }
        self.last_timestamp = trade.event_time;

        // 3. Check Outliers
        let price = trade.price.parse::<f64>().unwrap_or(0.0);
        if let Some(lp) = self.last_price {
            let diff = (price - lp).abs() / lp;
            if diff > self.outlier_threshold {
                self.outlier_count += 1;
                warn!("Filtered outlier trade price: {} (prev: {}, diff: {:.2}%)", price, lp, diff * 100.0);
                return false;
            }
        }
        self.last_price = Some(price);

        true
    }

    fn filter_agg_trade(&mut self, agg: &AggrTradesEvent) -> bool {
        // 1. Check Duplicates
        if agg.aggregated_trade_id <= self.last_agg_trade_id && self.last_agg_trade_id != 0 {
            self.duplicate_count += 1;
            warn!("Filtered duplicate aggTrade ID: {}", agg.aggregated_trade_id);
            return false;
        }
        self.last_agg_trade_id = agg.aggregated_trade_id;

        // 2. Check Out-of-order
        if agg.event_time < self.last_timestamp && self.last_timestamp != 0 {
            self.out_of_order_count += 1;
            warn!("Filtered out-of-order aggTrade: {} < {}", agg.event_time, self.last_timestamp);
            return false;
        }
        self.last_timestamp = agg.event_time;

        // 3. Check Outliers
        let price = agg.price.parse::<f64>().unwrap_or(0.0);
        if let Some(lp) = self.last_price {
            let diff = (price - lp).abs() / lp;
            if diff > self.outlier_threshold {
                self.outlier_count += 1;
                warn!("Filtered outlier aggTrade price: {} (prev: {}, diff: {:.2}%)", price, lp, diff * 100.0);
                return false;
            }
        }
        self.last_price = Some(price);

        true
    }

    pub fn get_quality_score(&self) -> f64 {
        if self.total_received == 0 { return 100.0; }
        let bad = self.duplicate_count + self.out_of_order_count + self.outlier_count;
        ((self.total_received - bad) as f64 / self.total_received as f64) * 100.0
    }
}
