#[derive(Debug)]
struct Transaction {
    from: String,
    to: String,
    value: u64,
    gas: u64,
}

#[derive(Debug)]
struct Block {
    number: u64,
    hash: String,
    transactions: Vec<Transaction>,
}

impl Block {
    fn scan_block(block_number: u64) -> Block {
        let tx1 = Transaction {
            from: "0xabc…".to_string(),
            to: "0xdef…".to_string(),
            value: 1000,
            gas: 21000,
        };

        let tx2 = Transaction {
            from: "0xghi…".to_string(),
            to: "0xjkl…".to_string(),
            value: 5000,
            gas: 45000,
        };

        Block {
            number: block_number,
            hash: format!("0xhash{}", block_number),
            transactions: vec![tx1, tx2],
        }
    }

    fn store_blocks(start: u64, end: u64) -> Vec<Block> {
        let mut blocks = Vec::new();

        for block_number in start..=end {
            let block = Block::scan_block(block_number);
            println!("Scanned block: {:?}", block);
            blocks.push(block);
        }

        blocks
    }

    fn get_latest_block_number() -> u64 {
        100
    }

    fn listen_for_new_blocks(mut last_scanned_block: u64) {
        loop {
            let latest_block_number = Self::get_latest_block_number();

            if latest_block_number > last_scanned_block {
                let new_block = Block::scan_block(latest_block_number);
                println!("New block detected: {:?}", new_block);
                last_scanned_block = latest_block_number;
            }

            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    }
}

fn main() {
    let latest_block_number = Block::get_latest_block_number();
    let blocks = Block::store_blocks(0, latest_block_number);

    println!("Scanned blocks: {:?}", blocks);

    Block::listen_for_new_blocks(latest_block_number);
}
