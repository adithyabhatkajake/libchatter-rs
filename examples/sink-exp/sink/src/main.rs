use clap::{load_yaml, App};
use tokio::{io::AsyncWriteExt, net::{TcpStream}};
use libp2p::futures::{StreamExt};
use tokio_util::codec::{FramedRead};
use std::{error::Error, time::SystemTime};
use std::fs::File;
use std::{io, io::BufRead};
use util::{new_dummy_tx, codec::block::Codec};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let yaml = load_yaml!("cli.yml");
    let m = App::from_yaml(yaml).get_matches();

    let mut total:u64 = 0;
    if let Some(x) = m.value_of("stop_blocks") {
        total = x.parse().expect("invalid stop blocks");
    } else if let Some(x) = m.value_of("block_size") {
        let bs:u64 = x.parse().expect("invalid block size");
        if let Some(y) = m.value_of("stop") {
            let total_cmds:u64 = y.parse().expect("invalid stop");
            total = if total_cmds % bs != 0 {
                (total_cmds/bs)+1
            } else {
                total_cmds/bs
            };
        } else {
            panic!("At least one of stop and block_size or stop blocks must be specified");
        }
    } else {
        panic!("At least one of stop and block_size or stop blocks must be specified");
    }
    let mut relays:Vec<String> = Vec::new();
    if let Some(ip) = m.value_of("relay") {
        relays.push(ip.to_string());
    } else if let Some(file) = m.value_of("servers") {
        // Read file line by line and treat them as ips
        let f = File::open(file).expect("failed to open the file");
        for line in io::BufReader::new(f).lines() {
            if let Ok(s) = line {
                relays.push(s.trim().to_string());
            }
        }
    } else {
        panic!("Please provide one of --servers or --relay");
    }
    let indicator = vec![1 as u8;1];
    let mut receivers = Vec::new();
    for addr in relays {
        let (rd, mut _wr) = TcpStream::connect(addr).await
            .expect("failed to connected one of the relays")
            .into_split();
        _wr.write(&indicator).await
            .expect("failed to write the indicator byte");
        let reader = FramedRead::new(rd, Codec::new());
        receivers.push(reader);
    }
    let mut times = Vec::new();
    // let latest = std::time::SystemTime::now();
    // times.push(latest);
    for _i in 0..total {
        for w in 0..receivers.len() {
            receivers[w].next().await
                .expect("Failed to get from one of the relayers")
                .expect("Failed to read");
        }
        // measure the time
        let time_now = SystemTime::now();
        times.push(time_now);
    }
    for i in 0..times.len() {
        if i == 0 {
            continue;
        }
        println!("Time:{:?}", times[i].duration_since(times[i-1]));
    }
    Ok(())
}
