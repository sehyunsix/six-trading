use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};
use std::env;

pub mod repository;

pub async fn establish_connection() -> Pool<Postgres> {
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to create pool")
}
