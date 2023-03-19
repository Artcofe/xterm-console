
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::SystemTime;

use futures_util::{SinkExt, StreamExt};
use hmac_sha256::HMAC;
use rust_decimal::prelude::RoundingStrategy;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{from_str, to_string};
use time::OffsetDateTime;
use tokio::sync::{broadcast, mpsc, Semaphore, SemaphorePermit};
use tokio::task::{yield_now, JoinHandle};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

// DD: Feel free to adjust the parameters up to you.

const MARKET_WS_URL: &'static str = "wss://stream.crypto.com/v2/market";
const USER_WS_URL: &'static str = "wss://stream.crypto.com/v2/user";
const API_KEY: &'static str = ""; // insert your api key here
const SECRET_KEY: &'static str = ""; // insert your secret key here
const GAIN_THRESHOLD: Decimal = dec!(1.001); // Execute chains having gain only above this threshold (fees are taken into account)
const DAY_VOLUME_THRESHOLD: f64 = 3500.0; // Execute chains only with trading pairs having more volume than this threshold
const CHAINS_APPROX_FRACTION: f32 = 1.0; // Coefficient to work only with a part of all built chains.

// DD: Most of the chains start on just a few currencies line USDT, USDC, BTC.
// Doesn't make sense to look for other ones as they don't have enough volume
// for you to execute the chains anyway.

const STARTING_CURRENCIES: [&str; 3] = ["USDT", "USDC", "BTC"];
const STARTING_BALANCE_USDT: Decimal = dec!(2.0);
const STARTING_BALANCE_USDC: Decimal = dec!(2.0);
const STARTING_BALANCE_BTC: Decimal = dec!(0.0001);
const TRADING_FEE: Decimal = dec!(0.99925);

const USER_MPSC_REQUEST_CAPACITY: usize = 10;
const USER_BROADCAST_RESPONSE_CAPACITY: usize = 2;

const MARKET_MPSC_REQUEST_CAPACITY: usize = 10;
const MARKET_BROADCAST_RESPONSE_CAPACITY: usize = 32;
const MARKET_BROADCAST_DISPATCH_CAPACITY: usize = 32;

const ARB_EXECUTOR_ORDER_TIMEOUT: u64 = 3000;
const ARB_EXECUTOR_PENDING_TIMEOUT: u64 = 180000;

// Turn on/off the actual trading.
const RESEARCH_MODE: bool = false;

// Tools to perform get requests from exchange

#[derive(Debug, Deserialize, Clone)]
pub struct Response<T> {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub method: String,
    #[serde(deserialize_with = "str_or_i64")]
    pub code: i64,
    #[serde(default)]
    pub result: Option<T>,
}

fn str_or_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StrOrU64<'a> {
        Str(&'a str),
        I64(i64),
    }

    Ok(match StrOrU64::deserialize(deserializer)? {
        StrOrU64::Str(v) => v.parse().unwrap_or(0), // Ignoring parsing errors
        StrOrU64::I64(v) => v,
    })
}

async fn get_exc<T: DeserializeOwned + Default>(client: &reqwest::Client, url: &str) -> Result<T, Box<dyn Error>> {
    let response = client.get(url).send().await?;
    let response_body: String = response.text().await?;
    let response_body_serialized: Response<T> = from_str(response_body.as_str())?;
    Ok(response_body_serialized.result.unwrap())
}

// Initial possible chains exploration and management

#[derive(Debug, Deserialize, Default)]
pub struct TickersData {
    pub data: Vec<TickerData>,
}

#[derive(Debug, Deserialize, Default)]
pub struct Instruments {
    pub instruments: Vec<Instrument>,
}

#[derive(Debug, Clone)]
pub struct ArbitrageChain {
    pub orders: [Order; 3],
    pub is_buys: [bool; 3],
    pub quantity_precisions: [usize; 3],
    pub starting_currency: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Instrument {
    pub instrument_name: String,
    pub quote_currency: String,
    pub base_currency: String,
    pub price_decimals: usize,
    pub quantity_decimals: usize,
    pub max_quantity: String,
    pub min_quantity: String,
}

impl Instruments {
    pub fn filter_day_vol(&mut self, tickers_data: Vec<TickerData>, day_volume_threshold: f64) {
        // DD: Filtering day volumes in some way is necessary due to pairs
        // with low volumes always contributing to some beefy gains but
        // not being actually executable (i.e. you can't immediately execute
        // deals on those pairs at the strict price points you've set)
        let mut instruments_filtered: Vec<Instrument> = Vec::new();
        for instrument in self.instruments.iter() {
            for ticker in tickers_data.iter() {
                if ticker.instrument.as_ref().unwrap() == &instrument.instrument_name && ticker.vol_traded_day_usd > day_volume_threshold {
                    instruments_filtered.push(instrument.clone());
                }
            }
        }
        self.instruments = instruments_filtered;
    }

    pub fn get_chains(&self, starting_currencies: Vec<&str>, approx_fraction: f32) -> (Vec<ArbitrageChain>, Vec<String>) {
        // DD: Computing the chains the dumbest way possible as it is
        // not a performance-sensitive part of the program. Returning
        // back the actual orders that have to be sent to the exchange.
        let mut arbitrage_chains: Vec<ArbitrageChain> = Vec::new();
        let mut instruments_all_chains: HashSet<String> = HashSet::new();

        for starting_currency in starting_currencies {
            for first_instrument in self
                .instruments
                .iter()
                .filter(|v| v.base_currency == starting_currency || v.quote_currency == starting_currency)
            {
                let mut new_chain: [String; 3] = ["".to_owned(), "".to_owned(), "".to_owned()];
                if first_instrument.quote_currency == starting_currency {
                    new_chain[0] = first_instrument.quote_currency.clone();
                    new_chain[1] = first_instrument.base_currency.clone();
                } else if first_instrument.base_currency == starting_currency {
                    new_chain[0] = first_instrument.base_currency.clone();
                    new_chain[1] = first_instrument.quote_currency.clone();
                } else {
                    panic!("starting instrument has to contain starting currency (most likely starting currency filtering is broken)");
                }

                for second_instrument in self.instruments.iter().filter(|v| v.instrument_name != first_instrument.instrument_name) {
                    let mut new_chain_second_level = new_chain.clone();
                    let asset_to_add: String;
                    if second_instrument.quote_currency == new_chain_second_level[1] {
                        asset_to_add = second_instrument.base_currency.clone();
                    } else if second_instrument.base_currency == new_chain_second_level[1] {
                        asset_to_add = second_instrument.quote_currency.clone();
                    } else {
                        continue;
                    }

                    let possible_third_instrument_name = format!("{}_{}", starting_currency, asset_to_add);
                    let possible_third_instrument_name_rev = format!("{}_{}", asset_to_add, starting_currency);

                    for third_instrument in self
                        .instruments
                        .iter()
                        .filter(|v| v.base_currency == starting_currency || v.quote_currency == starting_currency)
                    {
                        if (possible_third_instrument_name == third_instrument.instrument_name
                            || possible_third_instrument_name_rev == third_instrument.instrument_name)
                            && rand::random::<f32>() < approx_fraction
                        {
                            new_chain_second_level[2] = asset_to_add;

                            let chain_instruments: [Instrument; 3] = [first_instrument.clone(), second_instrument.clone(), third_instrument.clone()];
                            let mut sides: [String; 3] = ["".to_owned(), "".to_owned(), "".to_owned()];
                            let mut is_buys = [false; 3];
                            for idx in 0..3 {
                                instruments_all_chains.insert(chain_instruments[idx].instrument_name.clone());
                                if chain_instruments[idx].instrument_name.starts_with(&new_chain_second_level[idx]) {
                                    is_buys[idx] = false;
                                    sides[idx] = "SELL".to_owned();
                                } else {
                                    is_buys[idx] = true;
                                    sides[idx] = "BUY".to_owned();
                                }
                            }
                            let orders: [Order; 3] = core::array::from_fn(|i| Order {
                                instrument_name: chain_instruments[i].instrument_name.clone(),
                                side: sides[i].clone(),
                                type_: "LIMIT".to_owned(),
                                price: None,
                                quantity: None,
                                time_in_force: None,
                            });
                            let quantity_precisions: [usize; 3] = core::array::from_fn(|i| chain_instruments[i].quantity_decimals);

                            let arbitrage_chain = ArbitrageChain {
                                orders: orders,
                                is_buys: is_buys,
                                quantity_precisions: quantity_precisions,
                                starting_currency: starting_currency.to_owned(),
                            };

                            arbitrage_chains.push(arbitrage_chain);
                            break;
                        }
                    }
                    if new_chain_second_level[2] != "" {
                        continue;
                    }
                }
            }
        }
        let instruments_all_chains_vec: Vec<String> = instruments_all_chains.into_iter().collect();
        (arbitrage_chains, instruments_all_chains_vec)
    }
}

// Structs to serialize WebSocket request

#[derive(Serialize, Debug, Clone)]
#[serde(tag = "method")]
pub enum RequestWebSocket {
    #[serde(rename = "public/respond-heartbeat")]
    PublicRespondHeartbeat { id: u64 },
    #[serde(rename = "subscribe")]
    Subscribe { id: u64, nonce: u64, params: RequestWebSocketParams },
    #[serde(rename = "public/auth")]
    Auth { id: u64, api_key: String, sig: String, nonce: u64 },
    #[serde(rename = "private/create-order")]
    CreateOrder { id: i64, nonce: u64, params: Order },
    #[serde(rename = "private/cancel-all-orders")]
    CancelAllOrders { id: i64, nonce: u64, params: OrderCancellation },
}

#[derive(Serialize, Debug, Clone)]
pub enum RequestWebSocketParams {
    #[serde(rename = "channels")]
    Channels(Vec<String>),
}

#[derive(Debug, Serialize, Clone)]
pub struct Order {
    pub instrument_name: String,
    pub side: String,
    #[serde(rename(serialize = "type"))]
    pub type_: String,
    #[serde(with = "rust_decimal::serde::float_option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::float_option")]
    pub quantity: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct OrderCancellation {
    pub instrument_name: String,
}

// Structs to deserialize WebSocket responses

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "method")]
pub enum ResponseWebSocket {
    #[serde(rename = "public/heartbeat")]
    PublicHeartbeat { id: Option<u64>, code: Option<i64> },
    #[serde(rename = "public/auth")]
    Auth { id: u64, code: i64 },
    #[serde(rename = "subscribe")]
    Subscribe {
        id: Option<i64>,
        code: Option<i64>,
        result: Option<ResponseResult>,
    },
    #[serde(rename = "private/create-order")]
    CreateOrder {
        id: i64,
        code: i64,
        result: Option<OrderCreationResult>,
    },
    #[serde(rename = "private/cancel-all-orders")]
    CancelAllOrders { id: i64, code: i64 },
}

#[derive(Deserialize, Debug, Clone)]
pub struct OrderCreationResult {
    pub order_id: String,
    pub client_oid: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "channel")]
pub enum ResponseResult {
    #[serde(rename = "ticker")]
    Ticker { instrument_name: String, data: Vec<TickerData> },
    #[serde(rename = "user.order")]
    UserOrder { instrument_name: String, data: Vec<UserOrderData> },
}

#[derive(Deserialize, Debug, Clone)]
pub struct TickerData {
    #[serde(alias = "i")]