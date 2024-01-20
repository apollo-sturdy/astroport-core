use cosmwasm_schema::cw_serde;
use cosmwasm_std::Binary;

use crate::{asset::AssetInfo, factory::PairType};

/// This structure describes the parameters used for creating a contract.
#[cw_serde]
pub struct InstantiateMsg {
    /// The type of pair to create, e.g. Xyk, Stable, etc.
    pub pair_type: PairType,
    /// Information about assets in the pool
    pub asset_infos: Vec<AssetInfo>,
    /// The token contract code ID used for the tokens in the pool
    pub token_code_id: u64,
    /// The factory contract address
    pub factory_addr: String,
    /// The contract address of the cw20-adapter contract
    pub cw20_adapter_addr: String,
    /// Optional binary serialised parameters for custom pool types
    pub init_params: Option<Binary>,
}
