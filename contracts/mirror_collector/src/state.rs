use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use cosmwasm_std::{CanonicalAddr, StdResult, Storage};
use cosmwasm_storage::{singleton, singleton_read};

pub static KEY_CONFIG: &[u8] = b"config";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: CanonicalAddr,
    pub distribution_contract: CanonicalAddr, // collected rewards receiver
    pub daodiseoswap_factory: CanonicalAddr,     // daodiseoswap factory contract
    pub mirror_token: CanonicalAddr,
    pub base_denom: String,
    // aUST params
    pub aust_token: CanonicalAddr,
    pub anchor_market: CanonicalAddr,
    // bLuna params
    pub bluna_token: CanonicalAddr,
    // Lunax params
    pub lunax_token: CanonicalAddr,
    // when set, use this address instead of querying from daodiseoswap
    pub mir_ust_pair: Option<CanonicalAddr>,
}

pub fn store_config(storage: &mut dyn Storage, config: &Config) -> StdResult<()> {
    singleton(storage, KEY_CONFIG).save(config)
}

pub fn read_config(storage: &dyn Storage) -> StdResult<Config> {
    singleton_read(storage, KEY_CONFIG).load()
}
