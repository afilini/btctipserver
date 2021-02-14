#[macro_use]
extern crate log;
extern crate env_logger;
extern crate simple_server;
extern crate bdk;
extern crate serde_json;
extern crate bdk_macros;
extern crate ini;

use ini::Ini;

use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::str;

use bdk::sled;
use bdk::{Wallet};
use bdk::bitcoin::Address;

use simple_server::{Method, Server, StatusCode};
use bdk::electrum_client::{Client, ElectrumApi, ListUnspentRes};

fn prepare_home_dir() -> PathBuf {
    let mut dir = PathBuf::new();
    dir.push(&dirs_next::home_dir().unwrap());
    dir.push(".bdk-bitcoin");

    if !dir.exists() {
        info!("Creating home directory {}", dir.as_path().display());
        fs::create_dir(&dir).unwrap();
    }

    dir.push("database.sled");
    dir
}

fn new_address() -> Result<Address, bdk::Error> {
    let conf = Ini::load_from_file("config.ini").unwrap();

    let section_bdk = conf.section(Some("BDK")).unwrap();
    // let dir = section_bdk.get("datadir").unwrap();
    let descriptor = section_bdk.get("descriptor").unwrap();
    let change_descriptor = section_bdk.get("change_descriptor").unwrap();
    let network = section_bdk.get("network").unwrap();
    let wallet = section_bdk.get("wallet").unwrap();

    let database = sled::open(prepare_home_dir().to_str().unwrap()).unwrap();
    let tree = database.open_tree(wallet).unwrap();

    let wallet = Wallet::new_offline(
        descriptor.to_string().as_str(),
        Some(change_descriptor.to_string().as_str()),
        network.parse().unwrap(),
        tree,
    )?;

    let addr = wallet.get_new_address()?;
    Ok(addr)
}

fn check_address(addr: String, from_height: Option<usize>) -> Result<Vec<ListUnspentRes>, bdk::Error> {

    let conf = Ini::load_from_file("config.ini").unwrap();
    let section_bdk = conf.section(Some("BDK")).unwrap();
    let network = section_bdk.get("network").unwrap();
    let url = match network.parse().unwrap() {
        bdk::bitcoin::Network::Bitcoin => { "ssl://electrum.blockstream.info:50002" }
        bdk::bitcoin::Network::Testnet => { "ssl://electrum.blockstream.info:60002"}
        _ => { "" }
    };

    let client = Client::new(url).unwrap();

    let monitor_script = Address::from_str(addr.as_str())
        .unwrap()
        .script_pubkey();

    let unspents = client
        .script_list_unspent(&monitor_script)
        .unwrap();

    let array = unspents.into_iter()
        .filter(|x| x.height >= from_height.unwrap_or(0))
        .map(|x| x).collect();

    Ok(array)
}

fn html(address: String) -> Result<String, std::io::Error> {
    let list = check_address(address.as_str().to_string(), Option::from(0)).unwrap();
    let amount: u64 = list.iter().map(|x| x.value).sum();

    let mut status = format!("No onchain tx found yet");
    if amount > 0 {
        status = format!("Received {} sat", amount.to_string());
    }

    let template = fs::read_to_string("assets/index.html").unwrap();
    let link = format!("/bitcoin/?{}", address);
    let txt = template
        .replace("{address}", address.as_str())
        .replace("{status}", status.as_str())
        .replace("{refresh-link}", link.as_str())
        .replace("{refresh-timeout}", "30");
    Ok(txt)
}

fn redirect() -> Result<String, std::io::Error> {
    let address = new_address().unwrap();
    let link = format!("/bitcoin/?{}", address.to_string().as_str());
    let html = format!("<head><meta http-equiv=\"Refresh\" content=\"0; URL={}\"></head>", link);
    Ok(html)
}

fn main() {
    let host = "127.0.0.1";
    let port = "7878";

    let server = Server::new(|request, mut response| {
        println!("Request: {} {}", request.method(), request.uri());
        println!("Body: {}", str::from_utf8(request.body()).unwrap());
        println!("Headers:");
        for (key, value) in request.headers() {
            println!("{}: {}", key, value.to_str().unwrap());
        }

        match (request.method(), request.uri().path()) {
            (&Method::GET, "/bitcoin/api/new") => {
                // curl 127.0.0.1:7878/bitcoin/api/new
                let addr = new_address();
                //info!("addr {}", addr.to_string());
                return match addr {
                    Ok(a) => {
                        info!("new addr {}", a.to_string());
                        let value = serde_json::json!({
                                "network": a.network.to_string(),
                                "address": a.to_string()
                            });
                        Ok(response.body(value.to_string().as_bytes().to_vec())?)
                    },
                    Err(e) => Ok(response.body(e.to_string().as_bytes().to_vec())?)
                };
            }
            (&Method::GET, "/bitcoin/api/check") => {
                // curl 127.0.0.1:7878/bitcoin/api/check?tb1qm4safqvzu28jvjz5juta7qutfaqst7nsfsumuz:0
                let mut query = request.uri().query().unwrap_or("").split(":");
                let addr = query.next().unwrap();
                let height = query.next().unwrap();
                let h: usize = height.parse::<usize>().unwrap();

                let list = check_address(addr.to_string(), Option::from(h));
                return match list {
                    Ok(list) => {
                        println!("addr {} height {}", addr.to_string(), h.to_string());
                        for item in list.iter() {
                            println!("{} {}", item.value, item.height);
                            let value = serde_json::json!({
                                "value": item.value,
                                "height": item.height,
                                "tx_hash": item.tx_hash.to_string(),
                            });
                        }
                        Ok(response.body("".as_bytes().to_vec())?)
                    },
                    Err(e) => Ok(response.body(e.to_string().as_bytes().to_vec())?)
                }
            }
            (&Method::GET, "/bitcoin/") => {
                let address = request.uri().query().unwrap();
                return match html(address.to_string()) {
                    Ok(txt) => {
                        Ok(response.body(txt.as_bytes().to_vec())?)
                    },
                    Err(e) => Ok(response.body(e.to_string().as_bytes().to_vec())?)
                }
            }
            (&Method::GET, "/bitcoin") => {
                return match redirect() {
                    Ok(txt) => {
                        Ok(response.body(txt.as_bytes().to_vec())?)
                    },
                    Err(e) => Ok(response.body(e.to_string().as_bytes().to_vec())?)
                }
            }
            (_, _) => {
                println!("uri");
                response.status(StatusCode::NOT_FOUND);
                Ok(response.body("<h1>404</h1><p>Not found!<p>".as_bytes().to_vec())?)
            }
        }
    });

    server.listen(host, port);
}
