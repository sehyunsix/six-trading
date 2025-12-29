use super::MarketEvent;
use binance::websockets::*;
use log::{info, error, warn};
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc;

pub struct MarketDataManager {
    pub symbol: String,
    sender: mpsc::Sender<MarketEvent>,
}

impl MarketDataManager {
    pub fn new(symbol: String, sender: mpsc::Sender<MarketEvent>) -> Self {
        Self { symbol, sender }
    }

    pub async fn connect(&self) {
        let symbol = self.symbol.to_lowercase();
        let sender = self.sender.clone();

        info!("Connecting to market data for {}", symbol);

        // Use a dedicated standard thread to completely isolate 
        // the blocking binance-rs client from the tokio runtime.
        std::thread::spawn(move || {
            let keep_running = AtomicBool::new(true);
            let sender_clone = sender.clone();
            let symbol_log = symbol.clone();
            
            let web_socket = WebSockets::new(move |event: WebsocketEvent| {
                match event {
                    WebsocketEvent::Trade(trade) => {
                        // info!("WS Received Trade: {} Price={} Qty={}", trade.symbol, trade.price, trade.qty);
                        if let Err(e) = sender_clone.blocking_send(MarketEvent::Trade(trade)) {
                            error!("Failed to send trade event: {}", e);
                        }
                    }
                    WebsocketEvent::AggrTrades(agg) => {
                        // info!("WS Received AggrTrade: {} Price={} Qty={}", agg.symbol, agg.price, agg.qty);
                        if let Err(e) = sender_clone.blocking_send(MarketEvent::AggrTrade(agg)) {
                            error!("Failed to send aggTrade event: {}", e);
                        }
                    }
                    WebsocketEvent::OrderBook(depth) => {
                         // info!("WS Received OrderBook for {}", symbol_log);
                         if let Err(e) = sender_clone.blocking_send(MarketEvent::OrderBook(depth)) {
                            error!("Failed to send depth event: {}", e);
                         }
                    }
                    WebsocketEvent::DepthOrderBook(depth) => {
                         // info!("WS Received DepthUpdate for {}", symbol_log);
                         if let Err(e) = sender_clone.blocking_send(MarketEvent::DepthUpdate(depth)) {
                            error!("Failed to send depth event: {}", e);
                         }
                    }
                     _ => (),
                };
                Ok(())
            });

            // Leak web_socket to ensure its internal reqwest client 
            // is NEVER dropped during a tokio shutdown context.
            let web_socket = Box::leak(Box::new(web_socket));

            if let Err(e) = web_socket.connect_multiple_streams(&vec![
                format!("{}@trade", symbol),
                format!("{}@aggTrade", symbol),
                format!("{}@depth10@100ms", symbol),
            ]) {
                 error!("Failed to connect WS: {}", e);
                 return;
            }

            if let Err(e) = web_socket.event_loop(&keep_running) {
                 error!("Error in WS event loop: {}", e);
            }
            
            warn!("WS event loop exited");
        });
    }
}
