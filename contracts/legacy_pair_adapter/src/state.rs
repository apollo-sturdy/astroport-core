use astroport::asset::PairInfo;
use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::Item;

/// This structure stores the main config parameters for a constant product pair contract.
#[cw_serde]
pub struct Config {
    /// General pair information (e.g pair type)
    pub pair_info: PairInfo,
    /// The factory contract address
    pub factory_addr: Addr,
    /// The contract address of the cw20-adapter contract
    pub cw20_adapter_addr: Addr,
}

/// Stores the config struct at the given key
pub const CONFIG: Item<Config> = Item::new("config");

/// Stores the native token denom for the LP token of the underlying pair
pub const UNDERLYING_LP_TOKEN_DENOM: Item<String> = Item::new("lp_token_denom");

/// The underlying pool that this wrapper is wrapping
pub const UNDERLYING_POOL: Item<Addr> = Item::new("underlying_pool");
