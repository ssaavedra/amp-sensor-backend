// Requests the sqlite database as a parameter
// And makes it so that for every log entry from yesterday or before only the average of each minute is stored in the database

use sqlx::migrate::MigrateDatabase;
use sqlx::sqlite::SqlitePool;
use types::DbRow;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::process;
mod types;


/// Consolidate logs from the source database into the consolidated database.
/// 
/// This script will:
/// - Create the consolidated database if it does not exist.
/// - Ensure that all users and tokens from the source database exist in the consolidated database.
/// - Consolidate all logs from the source database into the consolidated database (without duplicates).
/// 
/// After this, the consolidated database will contain the same data as the source database, but with
/// logs consolidated by minute. You can then use the consolidated database for analysis.
/// 
/// You can delete old contents from the source database after running this script with the following SQL:
/// ```sql
/// DELETE FROM energy_log WHERE created_at < strftime('%s', 'now', '-1 day');
/// VACUUM;
/// ```
/// 
/// # Usage
/// 
/// ```sh
/// cargo run --bin consolidate_logs <sqlite database> <consolidated sqlite database>
/// ```
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!(
            "Usage: {} <sqlite database> <consolidated sqlite database>",
            args[0]
        );
        process::exit(1);
    }

    let db_path = Path::new(&args[1]);
    if !db_path.exists() {
        eprintln!("Error: {} does not exist", db_path.display());
        process::exit(1);
    }
    let db_consolidated_path = Path::new(&args[2]);

    rocket::tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async {
            if !sqlx::Sqlite::database_exists(db_consolidated_path.to_str().unwrap())
                .await
                .unwrap()
            {
                eprintln!(
                    "Error: {} does not exist yet. Creating.",
                    db_consolidated_path.display()
                );
                sqlx::Sqlite::create_database(&db_consolidated_path.to_str().unwrap())
                    .await
                    .unwrap();
            }
            let db_consolidated = SqlitePool::connect(&args[2]).await.unwrap();

            eprintln!("Ensuring migrations are up to date");
            sqlx::migrate!("./migrations")
                .run(&db_consolidated)
                .await
                .unwrap();
            eprintln!("Migrations complete. Database ready to use.");

            let db = SqlitePool::connect(&args[1]).await.unwrap();

            ensure_users_and_tokens_exist(&db, &db_consolidated)
                .await
                .expect("Error ensuring users and tokens exist");

            consolidate_logs(&db, &db_consolidated).await;
        });
}



async fn ensure_users_and_tokens_exist(
    db: &SqlitePool,
    db_consolidated: &SqlitePool,
) -> Result<(), sqlx::Error> {
    let users = sqlx::query!("SELECT * FROM users").fetch_all(db).await?;

    // Insert only those users that do not exist in the consolidated database
    let existing_users = sqlx::query!("SELECT id FROM users")
        .fetch_all(db_consolidated)
        .await?
        .iter()
        .map(|row| row.id.clone())
        .collect::<Vec<i64>>();

    for user in users {
        if existing_users.contains(&user.id) {
            continue;
        }
        sqlx::query!(
            "INSERT INTO users (id, location) VALUES (?, ?)",
            user.id,
            user.location,
        )
        .execute(db_consolidated)
        .await?;
    }

    let tokens = sqlx::query!("SELECT * FROM tokens").fetch_all(db).await?;
    let existing_tokens = sqlx::query!("SELECT token FROM tokens")
        .fetch_all(db_consolidated)
        .await?
        .iter()
        .map(|row| row.token.clone())
        .collect::<Vec<String>>();

    for token in tokens {
        if existing_tokens.contains(&token.token) {
            continue;
        }
        sqlx::query!(
            "INSERT INTO tokens (token, user_id) VALUES (?, ?)",
            token.token,
            token.user_id,
        )
        .execute(db_consolidated)
        .await?;
    }
    Ok(())
}

async fn consolidate_logs(db: &SqlitePool, db_consolidated: &SqlitePool) {
    let now = chrono::Utc::now();
    let yesterday = now - chrono::Duration::days(1);

    let old_logs: Vec<DbRow> = sqlx::query!("SELECT token, amps, volts, watts, created_at, user_agent, client_ip FROM energy_log WHERE created_at < ?", yesterday)
        .fetch_all(db)
        .await
        .unwrap().iter().map(|row| DbRow::new(
            row.token.clone(),
            row.amps,
            row.volts,
            row.watts,
            row.created_at,
            &row.user_agent,
            &row.client_ip,
        )).collect();

    let mut map = HashMap::new();
    let mut original_item_count = 0;

    for row in old_logs {
        let timestamp: i64 = row.created_at.timestamp();

        let minute = timestamp / 60;
        match map.entry(minute) {
            Entry::Occupied(mut entry) => {
                let s: &mut Vec<DbRow> = entry.get_mut();
                s.push(row);
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![row]);
            }
        }
        original_item_count += 1;
    }

    let map_len = map.len();

    // Add a unique constraint to prevent duplicates to (token, created_at)
    sqlx::query!("CREATE UNIQUE INDEX IF NOT EXISTS unique_token_created_at ON energy_log (token, created_at)")
        .execute(db_consolidated)
        .await
        .unwrap();

    for (minute, rows) in map {
        // Calculate the "average row"
        let rows_len = rows.len();
        let sum_rows: DbRow = rows.into_iter().sum();
        let avg_row = sum_rows / (rows_len as f64);

        // Insert the average row into the database
        let created_at = chrono::DateTime::<chrono::Utc>::from_timestamp(minute * 60, 0);
        let result = sqlx::query!(
            "INSERT INTO energy_log (token, amps, volts, watts, created_at, user_agent, client_ip) VALUES (?, ?, ?, ?, ?, ?, ?)",
            avg_row.token,
            avg_row.amps,
            avg_row.volts,
            avg_row.watts,
            created_at,
            "amp-consolidate-logs",
            avg_row.client_ip,
        ).execute(db_consolidated).await;

        match result {
            Ok(_) => {}
            Err(e)
                if e.as_database_error()
                    .is_some_and(|err| err.is_unique_violation()) =>
            {
                eprintln!(
                    "Preventing duplicate entry for token {} at {:#?}",
                    avg_row.token, created_at
                );
            }
            Err(e)
                if e.as_database_error()
                    .is_some_and(|err| err.is_foreign_key_violation()) =>
            {
                eprintln!("Token \"{}\" does not yet exist and was not migrated (did not exist either in the source DB). Automatically creating now and assigning to user_id=1. Please run again this script to include the missing row.", avg_row.token);
                sqlx::query!(
                    "INSERT INTO tokens (token, user_id) VALUES (?, ?)",
                    avg_row.token,
                    1,
                )
                .execute(db_consolidated)
                .await
                .unwrap();
            }
            Err(e) => {
                panic!("Error inserting row: {:?} for token {}", e, avg_row.token);
            }
        }
    }

    println!(
        "Consolidated {} entries into {} entries",
        original_item_count, map_len
    );

    println!("Total rows in the consolidated database: {}", sqlx::query!("SELECT COUNT(*) as count FROM energy_log")
        .fetch_one(db_consolidated)
        .await
        .unwrap()
        .count);
}
