use serde::{Deserialize, Serialize};
use sqlx::{Connection, Executor, PgConnection, PgPool};
use std::sync::Arc;
use web3::futures::StreamExt;
use web3::transports::WebSocket;
use web3::types::{Address, FilterBuilder, Log, H256, U256};
use web3::Web3;
use warp::Filter;
use std::str::FromStr;
use hex;

#[derive(Debug)]
enum Error {
    InvalidUserOpHash,
}

impl warp::reject::Reject for Error {}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserOperationEvent {
    user_op_hash: H256,
    sender: Address,
    paymaster: Address,
    nonce: U256,
    success: bool,
    actual_gas_cost: U256,
    actual_gas_used: U256,
}

async fn create_user_operations_table(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS user_operations (
            user_op_hash TEXT PRIMARY KEY,
            sender TEXT NOT NULL,
            paymaster TEXT NOT NULL,
            nonce NUMERIC NOT NULL,
            success BOOLEAN NOT NULL,
            actual_gas_cost NUMERIC NOT NULL,
            actual_gas_used NUMERIC NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn ensure_database_exists() -> Result<(), sqlx::Error> {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set in .env");

    let url = url::Url::parse(&database_url).expect("Invalid DATABASE_URL");
    let database_name = url.path().trim_start_matches('/');

    let mut url_no_db = url.clone();
    url_no_db.set_path("/postgres");
    let postgres_url = url_no_db.as_str();

    let mut conn = PgConnection::connect(postgres_url).await?;

    let db_exists = sqlx::query_scalar::<_, i32>("SELECT 1 FROM pg_database WHERE datname = $1")
        .bind(database_name)
        .fetch_optional(&mut conn)
        .await?
        .is_some();

    if !db_exists {
        println!("Database '{}' does not exist. Creating...", database_name);
        let create_db_query = format!("CREATE DATABASE \"{}\";", database_name);
        conn.execute(create_db_query.as_str()).await?;
        println!("Database '{}' created.", database_name);
    } else {
        println!("Database '{}' already exists.", database_name);
    }

    Ok(())
}

async fn query_by_user_op_hash(pool: &PgPool, user_op_hash: H256) -> Result<Option<UserOperationEvent>, sqlx::Error> {
    let user_op_hash_string = format!("{:?}", user_op_hash);

    let row = sqlx::query!(
        "SELECT user_op_hash, sender, paymaster, nonce, success, actual_gas_cost, actual_gas_used
         FROM user_operations
         WHERE user_op_hash = $1",
        user_op_hash_string
    )
    .fetch_optional(pool)
    .await?;

    if let Some(row) = row {
        let event = UserOperationEvent {
            user_op_hash: H256::from_str(&row.user_op_hash).unwrap(),
            sender: Address::from_str(&row.sender).unwrap(),
            paymaster: Address::from_str(&row.paymaster).unwrap(),
            nonce: U256::from_dec_str(&row.nonce).unwrap(), // Convert NUMERIC to U256
            success: row.success,
            actual_gas_cost: U256::from_dec_str(&row.actual_gas_cost).unwrap(),
            actual_gas_used: U256::from_dec_str(&row.actual_gas_used).unwrap(),
        };
        Ok(Some(event))
    } else {
        Ok(None)
    }
}

async fn query_by_sender(pool: &PgPool, sender: Address) -> Result<Vec<UserOperationEvent>, sqlx::Error> {
    let sender_string = format!("{:?}", sender);

    let rows = sqlx::query!(
        "SELECT user_op_hash, sender, paymaster, nonce, success, actual_gas_cost, actual_gas_used
         FROM user_operations
         WHERE sender = $1",
        sender_string
    )
    .fetch_all(pool)
    .await?;

    let events = rows
        .into_iter()
        .map(|row| UserOperationEvent {
            user_op_hash: H256::from_str(&row.user_op_hash).unwrap(),
            sender: Address::from_str(&row.sender).unwrap(),
            paymaster: Address::from_str(&row.paymaster).unwrap(),
            nonce: U256::from_dec_str(&row.nonce).unwrap(),
            success: row.success,
            actual_gas_cost: U256::from_dec_str(&row.actual_gas_cost).unwrap(),
            actual_gas_used: U256::from_dec_str(&row.actual_gas_used).unwrap(),
        })
        .collect();

    Ok(events)
}

async fn query_last_user_operations(pool: &PgPool) -> Result<Vec<UserOperationEvent>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT user_op_hash, sender, paymaster, nonce, success, actual_gas_cost, actual_gas_used
         FROM user_operations
         ORDER BY nonce DESC
         LIMIT 10"
    )
    .fetch_all(pool)
    .await?;

    let events = rows
        .into_iter()
        .map(|row| UserOperationEvent {
            user_op_hash: H256::from_str(&row.user_op_hash).unwrap(),
            sender: Address::from_str(&row.sender).unwrap(),
            paymaster: Address::from_str(&row.paymaster).unwrap(),
            nonce: U256::from_dec_str(&row.nonce).unwrap(),
            success: row.success,
            actual_gas_cost: U256::from_dec_str(&row.actual_gas_cost).unwrap(),
            actual_gas_used: U256::from_dec_str(&row.actual_gas_used).unwrap(),
        })
        .collect();

    Ok(events)
}

async fn start_api(pool: Arc<PgPool>) {
    let pool_filter = warp::any().map(move || pool.clone());

    let query_user_op_hash = warp::path!("user_op_hash" / String)
        .and(pool_filter.clone())
        .and_then(|user_op_hash: String, pool: Arc<PgPool>| async move {
            let user_op_hash_h256 = match H256::from_str(&user_op_hash) {
                Ok(hash) => hash,
                Err(err) => {
                    eprintln!("Invalid user_op_hash: {:?}", err);
                    return Err(warp::reject::custom(Error::InvalidUserOpHash));
                }
            };

            match query_by_user_op_hash(&pool, user_op_hash_h256).await {
                Ok(Some(event)) => Ok(warp::reply::json(&event)),
                Ok(None) => Err(warp::reject::not_found()),
                Err(err) => {
                    eprintln!("Error querying by user_op_hash: {:?}", err);
                    Err(warp::reject())
                }
            }
        });

    let query_last_op = warp::path!("last_user_operations")
        .and(pool_filter.clone())
        .and_then(|pool: Arc<PgPool>| async move {
            match query_last_user_operations(&pool).await {
                Ok(events) => Ok(warp::reply::json(&events)),
                Err(err) => {
                    eprintln!("Error querying last user operations: {:?}", err);
                    Err(warp::reject())
                }
            }
        });

    let routes = query_user_op_hash.or(query_last_op);

    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}

async fn connect_to_database() -> PgPool {
    ensure_database_exists()
        .await
        .expect("Failed to ensure database exists");

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set in .env");
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to the database");

    create_user_operations_table(&pool)
        .await
        .expect("Failed to create user_operations table");

    pool
}

async fn listen_user_operation_events(
    web3: Arc<Web3<WebSocket>>,
    pool: PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let entry_point_address: Address = "0x0000000071727de22e5e9d8baf0edac6f37da032"
        .parse()
        .unwrap();

    let user_op_event_topic: H256 =
        "0x49628fd1471006c1482da88028e9ce4dbb080b815c9b0344d39e5a8e6ec1419f"
            .parse()
            .unwrap();

    let filter = FilterBuilder::default()
        .address(vec![entry_point_address])
        .topics(Some(vec![user_op_event_topic]), None, None, None)
        .build();

    let log_stream = web3.eth_subscribe().subscribe_logs(filter).await?;

    log_stream
        .for_each(|log| async {
            match log {
                Ok(log) => {
                    if let Ok(event) = decode_user_operation_event(log) {
                        println!("New event detected: {:?}", event);
                        if let Err(err) = save_event_to_db(&pool, event).await {
                            eprintln!("Failed to save event to database: {:?}", err);
                        }
                    }
                }
                Err(err) => eprintln!("Error: {:?}", err),
            }
        })
        .await;

    Ok(())
}

fn decode_user_operation_event(log: Log) -> Result<UserOperationEvent, Box<dyn std::error::Error>> {
    let data = log.data.0.as_slice();
    let user_op_event = UserOperationEvent {
        user_op_hash: log.topics[1].into(),
        sender: log.topics[2].into(),
        paymaster: log.topics[3].into(),
        nonce: U256::from_big_endian(&data[0..32]),
        success: data[32] != 0,
        actual_gas_cost: U256::from_big_endian(&data[33..65]),
        actual_gas_used: U256::from_big_endian(&data[65..97]),
    };

    Ok(user_op_event)
}

async fn fetch_historical_events(
    web3: Arc<Web3<WebSocket>>,
    from_block: u64,
    to_block: u64,
    entry_point_address: Address,
    user_op_event_topic: H256,
) -> Result<Vec<Log>, Box<dyn std::error::Error>> {
    let filter = FilterBuilder::default()
        .address(vec![entry_point_address])
        .topics(Some(vec![user_op_event_topic]), None, None, None)
        .from_block(from_block.into())
        .to_block(to_block.into())
        .build();

    let logs = web3.eth().logs(filter).await?;
    Ok(logs)
}

async fn fetch_and_save_historical_events(
    web3: Arc<Web3<WebSocket>>,
    pool: &PgPool,
    from_block: u64,
    to_block: u64,
    entry_point_address: Address,
    user_op_event_topic: H256,
) -> Result<(), Box<dyn std::error::Error>> {
    let logs = fetch_historical_events(web3.clone(), from_block, to_block, entry_point_address, user_op_event_topic).await?;

    for log in logs {
        if let Ok(event) = decode_user_operation_event(log) {
            if let Err(err) = save_event_to_db(pool, event).await {
                eprintln!("Failed to save historical event to database: {:?}", err);
            }
        }
    }

    println!("Fetched and saved historical events from blocks {} to {}.", from_block, to_block);

    Ok(())
}

async fn save_event_to_db(pool: &PgPool, event: UserOperationEvent) -> Result<(), sqlx::Error> {
    let nonce = event.nonce.to_string();
    let actual_gas_cost = event.actual_gas_cost.to_string();
    let actual_gas_used = event.actual_gas_used.to_string();

    println!(
        "Inserting into user_operations: {:?}, {:?}, {:?}, {}, {}, {}, {}",
        event.user_op_hash,
        event.sender,
        event.paymaster,
        nonce,
        event.success,
        actual_gas_cost,
        actual_gas_used
    );

    let result = sqlx::query(
        "INSERT INTO user_operations (user_op_hash, sender, paymaster, nonce, success, actual_gas_cost, actual_gas_used)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (user_op_hash) DO NOTHING",
    )
    .bind(format!("{:?}", event.user_op_hash)) 
    .bind(format!("{:?}", event.sender))     
    .bind(format!("{:?}", event.paymaster))  
    .bind(nonce)                      
    .bind(event.success)              
    .bind(actual_gas_cost) 
    .bind(actual_gas_used)               
    .execute(pool)
    .await?;

    println!("Rows affected: {}", result.rows_affected());

    if result.rows_affected() == 0 {
        eprintln!("Event already exists or could not be saved: {:?}", event);
    } else {
        println!("Event successfully saved to the database.");
    }

    Ok(())
}

async fn connect_to_ethereum_node() -> Arc<Web3<WebSocket>> {
    let provider_url = std::env::var("ETH_NODE_URL").expect("ETH_NODE_URL must be set in .env");
    let ws = WebSocket::new(&provider_url)
        .await
        .expect("Failed to create WebSocket transport");
    let web3 = Web3::new(ws);
    Arc::new(web3)
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    let pool = connect_to_database().await;
    let web3 = connect_to_ethereum_node().await;

    let from_block = 0;
    let to_block = 100000;
    let entry_point_address = "0x0000000071727de22e5e9d8baf0edac6f37da032".parse().unwrap();
    let user_op_event_topic = "0x49628fd1471006c1482da88028e9ce4dbb080b815c9b0344d39e5a8e6ec1419f".parse().unwrap();

    let pool_for_api = pool.clone();
    tokio::spawn(async move {
        start_api(Arc::new(pool_for_api)).await;
    });

    if let Err(err) = fetch_and_save_historical_events(web3.clone(), &pool, from_block, to_block, entry_point_address, user_op_event_topic).await {
        eprintln!("Error fetching historical events: {:?}", err);
    }

    if let Err(err) = listen_user_operation_events(web3, pool).await {
        eprintln!("Error listening for events: {:?}", err);
    }
}