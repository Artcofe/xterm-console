
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