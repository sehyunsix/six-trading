-- Create trades table
CREATE TABLE IF NOT EXISTS trades (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_time BIGINT NOT NULL,
    symbol VARCHAR(20) NOT NULL,
    trade_id BIGINT NOT NULL,
    price DECIMAL NOT NULL,
    quantity DECIMAL NOT NULL,
    buyer_order_id BIGINT NOT NULL,
    seller_order_id BIGINT NOT NULL,
    is_buyer_maker BOOLEAN NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Create order_books table (snapshots)
CREATE TABLE IF NOT EXISTS order_books (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    last_update_id BIGINT NOT NULL,
    symbol VARCHAR(20) NOT NULL,
    bids JSONB NOT NULL,
    asks JSONB NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Index for backtesting range queries
CREATE INDEX IF NOT EXISTS idx_trades_event_time ON trades (event_time);
CREATE INDEX IF NOT EXISTS idx_order_books_created_at ON order_books (created_at);
