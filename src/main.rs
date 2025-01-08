use serde::{Deserialize, Serialize};
use std::sync::Arc;
use warp::Filter;
use web3::futures::StreamExt;
use web3::transports::WebSocket;
use web3::types::{Address, FilterBuilder, Log, H256, U256};
use web3::Web3;

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

async fn listen_user_operation_events(
    web3: Arc<Web3<WebSocket>>,
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

async fn connect_to_ethereum_node() -> Arc<Web3<WebSocket>> {
    let provider_url = std::env::var("ETH_NODE_URL").expect("ETH_NODE_URL must be set in .env");
    let ws = WebSocket::new(&provider_url)
        .await
        .expect("Failed to create WebSocket transport");
    let web3 = Web3::new(ws);
    Arc::new(web3)
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

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    let web3 = connect_to_ethereum_node().await;

    if let Err(err) = listen_user_operation_events(web3).await {
        eprintln!("Error listening for events: {:?}", err);
    }
}
