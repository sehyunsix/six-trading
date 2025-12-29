use binance::market::*;
use binance::futures::market::*;
use binance::api::*;
use binance::model::AggrTradesEvent;
use sqlx::{Pool, Postgres};
use log::{info, error, warn};
use crate::database::repository;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarketType {
    Spot,
    Futures,
}

impl MarketType {
    pub fn as_str(&self) -> &str {
        match self {
            MarketType::Spot => "SPOT",
            MarketType::Futures => "FUTURES",
        }
    }
}

pub struct HistoricalDownloader {
    pool: Pool<Postgres>,
}

impl HistoricalDownloader {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    pub async fn ensure_data(&self, symbol: &str, market_type: MarketType, hours: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        
        let target_start = now - (hours * 3600 * 1000);
        let market_str = market_type.as_str();
        
        info!("Ensuring at least {} hours of data for {} ({}) (targeting start: {})", hours, symbol, market_str, target_start);

        // Check the oldest record in DB for this symbol/market
        let oldest: Option<i64> = sqlx::query_scalar("SELECT MIN(event_time) FROM trades WHERE symbol = $1 AND market_type = $2")
            .bind(symbol)
            .bind(market_str)
            .fetch_optional(&self.pool)
            .await?
            .flatten();

        let start_from = match oldest {
            Some(ts) if (ts as u64) <= target_start => {
                info!("Database already contains data spanning the requested {} hours for {} ({}).", hours, symbol, market_str);
                return Ok(());
            }
            Some(ts) => ts as u64,
            None => now,
        };

        // We need to fetch from target_start up to start_from
        self.fetch_and_save_range_public(symbol, market_type, target_start, start_from).await?;

        Ok(())
    }

    /// Ensure data exists for a specific timestamp range (for backtesting)
    pub async fn ensure_data_range(&self, symbol: &str, market_type: MarketType, start_ts: u64, end_ts: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let market_str = market_type.as_str();
        info!("Checking data availability for {} ({}) from {} to {}", symbol, market_str, start_ts, end_ts);
        
        // Check existing data bounds
        let (db_min, db_max): (Option<i64>, Option<i64>) = sqlx::query_as(
            "SELECT MIN(event_time), MAX(event_time) FROM trades WHERE symbol = $1 AND market_type = $2"
        )
            .bind(symbol)
            .bind(market_str)
            .fetch_optional(&self.pool)
            .await?
            .unwrap_or((None, None));
        
        let need_before = match db_min {
            Some(min) if (min as u64) <= start_ts => false,
            _ => true,
        };
        
        let need_after = match db_max {
            Some(max) if (max as u64) >= end_ts => false,
            _ => true,
        };
        
        if !need_before && !need_after {
            info!("Database already has data covering the requested range for {} ({})", symbol, market_str);
            return Ok(());
        }
        
        let existing_min = db_min.map(|v| v as u64).unwrap_or(end_ts);
        let existing_max = db_max.map(|v| v as u64).unwrap_or(start_ts);
        
        // Download data before existing range if needed
        if need_before && start_ts < existing_min {
            info!("Downloading historical data BEFORE existing data: {} to {}", start_ts, existing_min);
            self.fetch_and_save_range_public(symbol, market_type, start_ts, existing_min).await?;
        }
        
        // Download data after existing range if needed  
        if need_after && end_ts > existing_max {
            info!("Downloading historical data AFTER existing data: {} to {}", existing_max, end_ts);
            self.fetch_and_save_range_public(symbol, market_type, existing_max, end_ts).await?;
        }
        
        Ok(())
    }

    pub async fn fetch_and_save_range_public(&self, symbol: &str, market_type: MarketType, start_ts: u64, end_ts: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use futures::stream::{self, StreamExt};
        use std::sync::Arc;
        use tokio::sync::Semaphore;
        
        let symbol_owned = symbol.to_string();
        let market_str = market_type.as_str().to_string();
        
        // Binance aggTrades API limits startTime-endTime window to 1 hour (3600000 ms)
        const MAX_WINDOW_MS: u64 = 3600000;
        const MAX_CONCURRENT_REQUESTS: usize = 5; // Limit concurrent requests to avoid rate limiting (429)
        
        // Calculate all chunks
        let mut chunks: Vec<(u64, u64)> = Vec::new();
        let mut chunk_start = start_ts;
        while chunk_start < end_ts {
            let chunk_end = std::cmp::min(chunk_start + MAX_WINDOW_MS, end_ts);
            chunks.push((chunk_start, chunk_end));
            chunk_start = chunk_end;
        }
        
        let total_chunks = chunks.len();
        info!("Fetching historical agg_trades for {} ({}) from {} to {} ({} chunks, {} concurrent)", 
              symbol, market_str, start_ts, end_ts, total_chunks, MAX_CONCURRENT_REQUESTS);
        
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
        let pool = self.pool.clone();
        
        // Process chunks in parallel with controlled concurrency
        let results: Vec<_> = stream::iter(chunks)
            .map(|(cs, ce)| {
                let sym = symbol_owned.clone();
                let market_str = market_str.clone();
                let pool = pool.clone();
                let semaphore = semaphore.clone();
                
                async move {
                    let _permit = semaphore.acquire().await.unwrap();
                    
                    // Stagger requests to avoid burst (Binance rate limit: 1200/min)
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    
                    let sym_clone = sym.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        match market_type {
                            MarketType::Spot => {
                                let market: Market = Binance::new(None, None);
                                market.get_agg_trades(&sym_clone, None, Some(cs), Some(ce), Some(1000))
                                    .map(|trades| trades.into_iter().map(|t| AggrTradesEvent {
                                        event_type: "aggTrade".to_string(),
                                        event_time: t.time,
                                        symbol: sym_clone.clone(),
                                        aggregated_trade_id: t.agg_id,
                                        price: t.price.to_string(),
                                        qty: t.qty.to_string(),
                                        first_break_trade_id: t.first_id,
                                        last_break_trade_id: t.last_id,
                                        trade_order_time: t.time,
                                        is_buyer_maker: t.maker,
                                        m_ignore: true,
                                    }).collect::<Vec<AggrTradesEvent>>())
                            }
                            MarketType::Futures => {
                                let market: FuturesMarket = Binance::new(None, None);
                                market.get_agg_trades(&sym_clone, None, Some(cs), Some(ce), Some(1000))
                                    .map(|trades_obj| {
                                        use binance::futures::model::AggTrades;
                                        match trades_obj {
                                            AggTrades::AllAggTrades(v) => v.into_iter().map(|t| AggrTradesEvent {
                                                event_type: "aggTrade".to_string(),
                                                event_time: t.time,
                                                symbol: sym_clone.clone(),
                                                aggregated_trade_id: t.agg_id,
                                                price: t.price.to_string(),
                                                qty: t.qty.to_string(),
                                                first_break_trade_id: t.first_id,
                                                last_break_trade_id: t.last_id,
                                                trade_order_time: t.time,
                                                is_buyer_maker: t.maker,
                                                m_ignore: true,
                                            }).collect::<Vec<AggrTradesEvent>>(),
                                        }
                                    })
                            }
                        }
                    }).await;
                    
                    match result {
                        Ok(Ok(events)) if !events.is_empty() => {
                            let count = events.len();
                            if let Err(e) = repository::save_aggr_trades_bulk(&pool, &events, &market_str).await {
                                error!("Failed to save chunk {}-{}: {}", cs, ce, e);
                            } else {
                                info!("Saved {} trades for chunk {}-{} ({})", count, cs, ce, sym);
                            }
                        }
                        Ok(Ok(_)) => {
                            // Empty chunk, skip
                        }
                        Ok(Err(e)) => {
                            error!("Binance API error for chunk {}-{}: {:?}", cs, ce, e);
                        }
                        Err(e) => {
                            error!("Task error for chunk {}-{}: {:?}", cs, ce, e);
                        }
                    }
                }
            })
            .buffer_unordered(MAX_CONCURRENT_REQUESTS)
            .collect()
            .await;

        info!("Historical data download complete for {} ({}) - processed {} chunks", symbol_owned, market_str, total_chunks);
        Ok(())
    }
}
