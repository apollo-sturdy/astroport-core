use crate::observation::Observation;
use cosmwasm_schema::{cw_serde, QueryResponses};
use cw_ownable::cw_ownable_execute;

use cosmwasm_std::{Addr, Binary, Coin, Decimal, Empty, Uint128, Uint64};

/// This structure describes the parameters used for creating a contract.
///
/// Generics:
/// - `P` - The parameters unique to the current pool type
#[cw_serde]
pub struct InstantiateMsg<P = Empty> {
    /// The token denoms of the assets in the pool
    pub reserve_denoms: Vec<String>,
    /// The factory contract address
    pub factory_addr: String,
    /// Optional parameters specific to the pool type
    pub init_params: Option<P>,
}

#[cw_serde]
pub struct FlashLoanHookMsg {
    /// This vec contains options for repaying the loan. Each coin in the vec contains the amount
    /// and denom of a token that can be used to pay back the loan.
    pub required_payment: Vec<Coin>,
    /// An optional binary encoded message passed to the calling contract.
    pub msg: Option<Binary>,
}

#[cw_serde]
pub struct Price {
    /// The denom of the asset to query the price for
    pub base_asset: String,
    /// The denom of the asset to quote the price in
    pub quote_asset: String,
    /// The price of the base asset quoted in terms of the quote asset. I.e. the number of quote
    /// assets per base asset.
    pub price: Decimal,
}

#[cw_serde]
/// Protects the user from slippage by ensuring that the price of the pool does not move too much.
/// If the execution price of the action is more than `slippage_tolerance` percent different
/// from the price supplied in the `belief_price` field, the transaction will revert.
pub struct SlippageControl {
    /// The user's belief of the price of the pool before the action.
    pub belief_price: Price,
    /// The maximum amount of slippage that is allowed.
    pub slippage_tolerance: Decimal,
}

/// This structure describes the execute messages available in the contract.
///
/// Generics:
/// - `U` - The config update messages specific to the pool type
#[cw_ownable_execute]
#[cw_serde]
pub enum ExecuteMsg<U = Empty> {
    /// Provides liquidity to the pool with the native tokens sent to the contract.
    /// Only those tokens that are already in the pool can be provided. If any additional tokens
    /// are sent, the transaction will revert.
    ProvideLiquidity {
        /// Determines whether the LP tokens minted for the user is auto_staked in the Generator contract
        auto_stake: Option<bool>,
        /// The recipient of the minted LP tokens
        recipient: Option<String>,
        /// The minimum amount of LP tokens to receive. If the amount received is less than this,
        /// the transaction will revert.
        min_out: Option<Uint128>,
        /// Optional parameters for price based slippage control.
        slippage_control: Option<SlippageControl>,
    },

    /// Withdraws liquidity from the pool. LP tokens should be sent along with the message to the contract.
    WithdrawLiquidity {
        /// The minimum amount of each asset to receive from the pool. If the amount received is
        /// less than this, the transaction will revert.
        min_out: Vec<Coin>,
        /// Optional parameters for price based slippage control.
        slippage_control: Option<SlippageControl>,
    },

    /// Swaps all the native tokens sent to the contract for the asset specified with the `ask_denom` field.
    SwapExactIn {
        /// The asset to receive from the swap
        ask_denom: String,
        /// The address to send the swapped tokens to. If not specified, the tokens will be sent to the caller.
        recipient: Option<String>,
        /// The minimum amount of `ask_denom` to receive from the swap. If the amount received is
        /// less than this, the transaction will revert.
        min_out: Option<Uint128>,
        /// Optional parameters for price based slippage control.
        slippage_control: Option<SlippageControl>,
    },

    /// Swaps some amount of the sent native tokens for exactly the amount and denom specified in the `ask` field.
    /// Any remaining unused tokens will be sent back to the sender.
    SwapExactOut {
        /// The asset to receive from the swap
        ask: Coin,
        /// The address to send the swapped tokens to. If not specified, the tokens will be sent to the caller.
        recipient: Option<String>,
        /// The maximum amount of native tokens to offer for the swap. If the amount needed to
        /// receive the requested asset is greater than this, the transaction will revert.
        max_in: Option<Vec<Coin>>,
        /// Optional parameters for price based slippage control.
        slippage_control: Option<SlippageControl>,
    },

    /// Borrows the requested tokens from the pool to the calling contract. The tokens will be sent
    /// to the calling contract (or the contract specified in the `recipient_contract` field) as part
    /// of a contract execution with ExecuteMsg `FlashLoanReceive(FlashLoanHookMsg)`.
    FlashLoan {
        /// The asset to receive as a loan
        receive: Coin,
        /// The contract which should receive the borrowed funds. This is the contract on which
        /// `FlashLoanReceive` will be called. If not specified, the caller's address will be used.
        recipient_contract: Option<String>,
        /// An optional binary encoded message to be sent back to the calling contract. This will be
        /// included wrapped inside of the `FlashLoanHookMsg` that is sent back to the calling contract.
        msg: Option<Binary>,
    },

    /// Update the pool configuration
    UpdateConfig { updates: ConfigUpdates<U> },
}

/// Available pool types
#[derive(Eq)]
#[cw_serde]
#[non_exhaustive]
pub enum PoolType {
    /// XYK pool type
    Xyk {},
    /// Stable pool type
    Stable {},
    /// Passive Concentraced Liquidity pool type
    Pcl {},
    /// Custom pool type
    Custom(String),
}

/// This structure describes the query messages available in the contract.
///
/// Generics:
/// - `Q` - The query messages specific to the pool type
/// - `P` - The parameters unique to the current pool type
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg<Q = Empty, P = Empty> {
    /// Returns information about the pool in an object of type [`PoolInfoResponse`].
    #[returns(PoolInfoResponse)]
    PoolInfo {},

    /// Returns information about a pool in an object of type [`PoolResponse`].
    #[returns(PoolSateResponse)]
    PoolState {},

    /// Returns contract configuration settings in a custom [`ConfigResponse`] structure.
    #[returns(ConfigResponse<P>)]
    Config {},

    /// Simulates providing liquidity to the pool and returns the amount of LP tokens that would be received.
    #[returns(Coin)]
    SimulateProvideLiquidity {
        /// The assets to provide to the pool
        assets: Vec<Coin>,
        /// The pool reserves to use for the simulation. If not specified, the current reserves will be used.
        reserves: Option<Vec<Coin>>,
        /// The parameters unique to the current pool type to use for the simulation. If not specified, the current parameters will be used.
        params: Option<P>,
    },

    /// Simulates withdrawing liquidity from the pool and returns the amount of assets that would be received.
    #[returns(Vec<Coin>)]
    SimulateWithdrawLiquidity {
        /// The amount of LP tokens to withdraw
        amount: Uint128,
        /// The pool reserves to use for the simulation. If not specified, the current reserves will be used.
        reserves: Option<Vec<Coin>>,
        /// The parameters unique to the current pool type to use for the simulation. If not specified, the current parameters will be used.
        params: Option<P>,
    },

    /// Simulates a swap with exact amounts of offer assets and returns a `SimulateSwapResponse` object.
    #[returns(SimulateSwapResponse<P>)]
    SimulateSwapExactIn {
        /// The asset to receive from the swap
        ask_denom: String,
        /// The assets to swap for the asset specified in the `ask_denom` field.
        offer_assets: Vec<Coin>,
        /// The pool reserves to use for the simulation. If not specified, the current reserves will be used.
        reserves: Option<Vec<Coin>>,
        /// The parameters unique to the current pool type to use for the simulation. If not specified, the current parameters will be used.
        params: Option<P>,
    },

    /// Simulates a swap with an exact amount of an asset to receive and returns a `SimulateSwapResponse` object.
    #[returns(SimulateSwapResponse<P>)]
    SimulateSwapExactOut {
        /// The asset to receive from the swap
        ask: Coin,
        /// The asset to swap for the asset specified in the `ask` field.
        offer_denom: String,
        /// The pool reserves to use for the simulation. If not specified, the current reserves will be used.
        reserves: Option<Vec<Coin>>,
        /// The parameters unique to the current pool type to use for the simulation. If not specified, the current parameters will be used.
        params: Option<P>,
    },

    /// Queries the time weighted average price of the `base_denom` asset quoted in terms of number
    /// of `quote_denom` assets.
    #[returns(Price)]
    TwapPrice {
        /// The denom of the asset to query the price for
        base_denom: String,
        /// The denom of the asset to quote the price in
        quote_denom: String,
        /// The UNIX timestamp in seconds of the start of the time window
        start_time: u64,
        /// The UNIX timestamp in seconds of the end of the time window
        end_time: u64,
    },

    /// Queries all of the stored oracle observations
    #[returns(Vec<Observation>)]
    OracleObservations {
        /// How many observations to return. If not specified, the query will try to return all
        /// observations, although this fail if gas limits are exceeded.
        limit: Option<u32>,
        /// The UNIX timestamp in seconds after which to start returning observations
        start_after: Option<u64>,
    },

    /// Returns the reserves that were in the pool prior to the given block height
    #[returns(Vec<Coin>)]
    PoolReservesAtHeight { block_height: Uint64 },

    /// Queries specific to the pool type
    #[returns(Empty)]
    PoolTypeQueries(Q),
}

/// This structure stores the main parameters for an Astroport pool
#[cw_serde]
pub struct PoolInfoResponse {
    /// The token denoms of the assets in the pool
    pub reserve_denoms: Vec<String>,
    /// Pool contract address
    pub contract_addr: Addr,
    /// Pool LP token denom
    pub liquidity_token_denom: String,
    /// The pool type (xyk, stableswap, etc) available in [`PoolType`]
    pub pool_type: PoolType,
}

/// This struct is used to return a query result with the total amount of LP tokens and assets in a specific pool.
#[cw_serde]
pub struct PoolSateResponse {
    /// The assets in the pool together with asset amounts
    pub pool_reserves: Vec<Coin>,
    /// The total amount of LP tokens currently issued
    pub lp_token_supply: Coin,
}

/// This structure holds the parameters that are returned from a swap simulation response
#[cw_serde]
pub struct SimulateSwapResponse<T = Empty> {
    /// The assets sent to the pool for the swap
    pub offer_assets: Vec<Coin>,
    /// The asset received from the pool for the swap
    pub return_asset: Coin,
    /// The change in spot price caused by the swap, in percentage form
    pub price_impact: Decimal,
    /// The fees charged for the swap
    pub commission: Coin,
    /// The difference in percentage between the prior spot price and the execution price
    pub slippage: Decimal,
    /// The pool reserves after the swap
    pub reserves_after: Vec<Coin>,
    /// The parameters unique to the current pool type after the swap
    pub parameters_after: Option<T>,
}

/// This struct is used to return a query result with the general contract configuration.
#[cw_serde]
pub struct ConfigResponse<P = Empty> {
    /// The contract owner
    pub owner: Addr,
    /// The factory contract address
    pub factory_addr: Addr,
    /// The fee share parameters
    pub fee_share: Option<FeeShareConfig>,
    /// The parameters unique to the current pool type
    pub params: P,
}

#[cw_serde]
pub struct ConfigUpdates<P = Empty> {
    /// The contract owner
    pub owner: Option<String>,
    /// The factory contract address
    pub factory_addr: Option<String>,
    /// The fee share parameters
    pub fee_share: Option<FeeShareConfig>,
    /// The parameters unique to the current pool type
    pub params: Option<P>,
}

/// Holds the configuration for fee sharing
#[cw_serde]
pub struct FeeShareConfig {
    /// The fee shared with the address
    pub bps: u16,
    /// The share is sent to this address on every swap
    pub recipient: Addr,
}

/// This structure describes a migration message.
/// We currently take no arguments for migrations.
#[cw_serde]
pub struct MigrateMsg {}
