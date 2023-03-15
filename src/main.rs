
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