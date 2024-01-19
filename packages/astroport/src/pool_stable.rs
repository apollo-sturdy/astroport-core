use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Decimal, Empty};

use crate::pool_new::{ExecuteMsg, InstantiateMsg, QueryMsg};

/// Parameters unique to the Stable pool type.
#[cw_serde]
pub struct StablePoolParams {
    /// The current stableswap pool amplification
    pub amp: Decimal,
}

/// This enum stores the options available to start and stop changing a stableswap pool's amplification.
#[cw_serde]
pub enum StablePoolConfigUpdates {
    StartChangingAmp { next_amp: u64, next_amp_time: u64 },
    StopChangingAmp {},
}

pub type StableInstantiateMsg = InstantiateMsg<StablePoolParams>;

pub type StableExecuteMsg = ExecuteMsg<StablePoolConfigUpdates>;

pub type StableQueryMsg = QueryMsg<Empty, StablePoolParams>;
