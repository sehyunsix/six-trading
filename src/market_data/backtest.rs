use tokio::sync::mpsc;
use crate::market_data::MarketEvent;
use crate::database::repository;
use sqlx::{Pool, Postgres};
use log::info;

pub struct BacktestDataManager {
    symbol: String,
    tx: mpsc::Sender<MarketEvent>,
    pool: Pool<Postgres>,
}

impl BacktestDataManager {
    pub fn new(symbol: String, tx: mpsc::Sender<MarketEvent>, pool: Pool<Postgres>) -> Self {
        Self { symbol, tx, pool }
    }

    pub async fn run_backtest(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Starting backtest for {}...", self.symbol);
        
        let trades = repository::get_historical_trades_range(&self.pool, &self.symbol, "SPOT", None, None).await?;
        info!("Loaded {} trades for backtesting", trades.len());

        for trade in trades {
            if let Err(e) = self.tx.send(MarketEvent::Trade(trade)).await {
                log::error!("Failed to send backtest trade: {}", e);
                break;
            }
        }

        info!("Backtest data streaming complete.");
        Ok(())
    }
}
