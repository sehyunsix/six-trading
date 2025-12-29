-- Add market_type column to trades and order_books
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='trades' AND column_name='market_type') THEN
        ALTER TABLE trades ADD COLUMN market_type VARCHAR(10) DEFAULT 'SPOT';
    END IF;
    
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name='order_books' AND column_name='market_type') THEN
        ALTER TABLE order_books ADD COLUMN market_type VARCHAR(10) DEFAULT 'SPOT';
    END IF;
END $$;
