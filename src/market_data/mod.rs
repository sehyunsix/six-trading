pub mod websocket;
pub mod backtest;
pub mod filter;

pub mod downloader;

pub use downloader::HistoricalDownloader;
pub use websocket::MarketDataManager;
pub use filter::DataFilter;

use binance::model::{TradeEvent, DepthOrderBookEvent, OrderBook, AggrTradesEvent};

#[derive(Debug, Clone)]
pub enum MarketEvent {
    Trade(TradeEvent),
    AggrTrade(AggrTradesEvent),
    OrderBook(OrderBook),
    #[allow(dead_code)]
    DepthUpdate(DepthOrderBookEvent),
}
