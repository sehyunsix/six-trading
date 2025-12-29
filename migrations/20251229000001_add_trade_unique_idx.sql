-- Add unique constraint for trade deduplication
CREATE UNIQUE INDEX IF NOT EXISTS idx_trades_unique 
ON trades (trade_id, symbol, market_type);
