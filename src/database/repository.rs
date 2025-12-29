use sqlx::{Pool, Postgres};
use binance::model::{TradeEvent, OrderBook, AggrTradesEvent};
use serde_json::json;


pub async fn save_trade(pool: &Pool<Postgres>, event: &TradeEvent, market_type: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO trades (event_time, symbol, market_type, trade_id, price, quantity, buyer_order_id, seller_order_id, is_buyer_maker)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(event.event_time as i64)
    .bind(&event.symbol)
    .bind(market_type)
    .bind(event.trade_id as i64)
    .bind(event.price.parse::<f64>().unwrap_or(0.0))
    .bind(event.qty.parse::<f64>().unwrap_or(0.0))
    .bind(event.buyer_order_id as i64)
    .bind(event.seller_order_id as i64)
    .bind(event.is_buyer_maker)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn save_aggr_trade(pool: &Pool<Postgres>, event: &AggrTradesEvent, market_type: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO trades (event_time, symbol, market_type, trade_id, price, quantity, buyer_order_id, seller_order_id, is_buyer_maker)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(event.event_time as i64)
    .bind(&event.symbol)
    .bind(market_type)
    .bind(event.aggregated_trade_id as i64)
    .bind(event.price.parse::<f64>().unwrap_or(0.0))
    .bind(event.qty.parse::<f64>().unwrap_or(0.0))
    .bind(0i64) // dummy
    .bind(0i64) // dummy
    .bind(event.is_buyer_maker)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn save_aggr_trades_bulk(pool: &Pool<Postgres>, events: &[AggrTradesEvent], market_type: &str) -> Result<(), sqlx::Error> {
    if events.is_empty() {
        return Ok(());
    }

    let mut query_builder: sqlx::QueryBuilder<Postgres> = sqlx::QueryBuilder::new(
        "INSERT INTO trades (event_time, symbol, market_type, trade_id, price, quantity, buyer_order_id, seller_order_id, is_buyer_maker) "
    );

    query_builder.push_values(events.iter(), |mut b, event| {
        b.push_bind(event.event_time as i64)
            .push_bind(&event.symbol)
            .push_bind(market_type)
            .push_bind(event.aggregated_trade_id as i64)
            .push_bind(event.price.parse::<f64>().unwrap_or(0.0))
            .push_bind(event.qty.parse::<f64>().unwrap_or(0.0))
            .push_bind(0i64) // dummy
            .push_bind(0i64) // dummy
            .push_bind(event.is_buyer_maker);
    });

    // Note: If the unique index exists, duplicates will fail silently
    // If not, duplicates may be inserted but that's fine for historical data
    let query = query_builder.build();
    match query.execute(pool).await {
        Ok(_) => Ok(()),
        Err(e) => {
            // If it's a duplicate key error, that's fine - data already exists
            if e.to_string().contains("duplicate key") || e.to_string().contains("unique constraint") {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

pub async fn cleanup_old_data(pool: &Pool<Postgres>, hours: i64) -> Result<u64, sqlx::Error> {
    let threshold = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64 - (hours * 3600 * 1000);

    let res = sqlx::query("DELETE FROM trades WHERE event_time < $1")
        .bind(threshold)
        .execute(pool)
        .await?;
    
    let res2 = sqlx::query("DELETE FROM order_books WHERE last_update_id < $1")
        .bind(threshold / 1000) // Rough approximation for last_update_id if not timestamped
        .execute(pool)
        .await?;
    
    Ok(res.rows_affected() + res2.rows_affected())
}

pub async fn save_order_book(pool: &Pool<Postgres>, symbol: &str, book: &OrderBook, market_type: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO order_books (last_update_id, symbol, market_type, bids, asks)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(book.last_update_id as i64)
    .bind(symbol)
    .bind(market_type)
    .bind(json!(book.bids))
    .bind(json!(book.asks))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_historical_trades(pool: &Pool<Postgres>, symbol: &str) -> Result<Vec<TradeEvent>, sqlx::Error> {
    get_historical_trades_range(pool, symbol, "SPOT", None, None).await
}

pub async fn get_historical_trades_range(
    pool: &Pool<Postgres>, 
    symbol: &str,
    market_type: &str,
    start_time: Option<u64>,
    end_time: Option<u64>
) -> Result<Vec<TradeEvent>, sqlx::Error> {
    let mut query_str = String::from(
        r#"
        SELECT event_time, symbol, trade_id, price::TEXT, quantity::TEXT, buyer_order_id, seller_order_id, is_buyer_maker
        FROM trades
        WHERE symbol = $1 AND market_type = $2
        "#
    );

    let mut bind_idx = 3;
    if start_time.is_some() {
        query_str.push_str(&format!(" AND event_time >= ${}", bind_idx));
        bind_idx += 1;
    }
    if end_time.is_some() {
        query_str.push_str(&format!(" AND event_time <= ${}", bind_idx));
    }
    query_str.push_str(" ORDER BY event_time ASC");

    let mut query = sqlx::query(&query_str).bind(symbol).bind(market_type);
    if let Some(st) = start_time {
        query = query.bind(st as i64);
    }
    if let Some(et) = end_time {
        query = query.bind(et as i64);
    }

    let rows = query.fetch_all(pool).await?;

    let trades = rows.into_iter().map(|row| {
        use sqlx::Row;
        TradeEvent {
            event_type: "trade".to_string(),
            event_time: row.get::<i64, _>("event_time") as u64,
            symbol: row.get::<String, _>("symbol"),
            trade_id: row.get::<i64, _>("trade_id") as u64,
            price: row.get::<Option<String>, _>("price").unwrap_or_else(|| "0".to_string()),
            qty: row.get::<Option<String>, _>("quantity").unwrap_or_else(|| "0".to_string()),
            buyer_order_id: row.get::<i64, _>("buyer_order_id") as u64,
            seller_order_id: row.get::<i64, _>("seller_order_id") as u64,
            trade_order_time: row.get::<i64, _>("event_time") as u64,
            is_buyer_maker: row.get::<bool, _>("is_buyer_maker"),
            m_ignore: true,
        }
    }).collect();

    Ok(trades)
}

#[derive(Debug, serde::Serialize)]
pub struct AggregatedData {
    pub timestamp: i64,
    pub price: f64, // Close price
    pub volume: f64,
}

pub async fn get_aggregated_trades(
    pool: &Pool<Postgres>, 
    symbol: &str, 
    market_type: &str,
    interval: &str // "minute", "hour"
) -> Result<Vec<AggregatedData>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT 
            EXTRACT(EPOCH FROM date_trunc($1, to_timestamp(event_time::FLOAT8 / 1000.0)))::BIGINT as bucket,
            (array_agg(price::FLOAT8 ORDER BY event_time DESC))[1] as close,
            SUM(quantity::FLOAT8) as volume
        FROM trades
        WHERE symbol = $2 AND market_type = $3
        GROUP BY bucket
        ORDER BY bucket ASC
        "#,
    )
    .bind(interval)
    .bind(symbol)
    .bind(market_type)
    .fetch_all(pool)
    .await?;

    let data = rows.into_iter().map(|row| {
        use sqlx::Row;
        AggregatedData {
            timestamp: row.get::<i64, _>("bucket"),
            price: row.get::<f64, _>("close"),
            volume: row.get::<f64, _>("volume"),
        }
    }).collect();

    Ok(data)
}

pub async fn get_data_range(pool: &Pool<Postgres>, symbol: &str, market_type: &str) -> Result<(Option<u64>, Option<u64>), sqlx::Error> {
    let row: (Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT MIN(event_time), MAX(event_time) FROM trades WHERE symbol = $1 AND market_type = $2"
    )
    .bind(symbol)
    .bind(market_type)
    .fetch_one(pool)
    .await?;

    Ok((row.0.map(|v| v as u64), row.1.map(|v| v as u64)))
}
