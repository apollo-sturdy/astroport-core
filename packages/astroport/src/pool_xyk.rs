use cosmwasm_schema::cw_serde;
use cosmwasm_std::Empty;

use crate::pool_new::{ExecuteMsg, InstantiateMsg, QueryMsg};

/// Parameters unique to the XYK pool type.
#[cw_serde]
pub struct XYKPoolParams {
    /// Whether asset balances are tracked over blocks or not.
    pub track_asset_balances: bool,
}

/// This enum stores the option available to enable asset balances tracking over blocks.
#[cw_serde]
pub enum XYKPoolConfigUpdates {
    /// Enables asset balances tracking over blocks.
    EnableAssetBalancesTracking,
}

pub type XykInstantiateMsg = InstantiateMsg<XYKPoolParams>;

pub type XykExecuteMsg = ExecuteMsg<XYKPoolConfigUpdates>;

pub type XykQueryMsg = QueryMsg<Empty, XYKPoolParams>;
