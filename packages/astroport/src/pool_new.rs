use crate::observation::OracleObservation;
use cosmwasm_schema::{cw_serde, QueryResponses};
use cw_ownable::cw_ownable_execute;

use cosmwasm_std::{coin, Addr, Binary, Coin, Decimal, Decimal256, Empty, Uint128, Uint64};

/// The default swap slippage
pub const DEFAULT_SLIPPAGE: &str = "0.005";
/// The maximum allowed swap slippage
pub const MAX_ALLOWED_SLIPPAGE: &str = "0.5";
/// The maximum fee share allowed, 10%
pub const MAX_FEE_SHARE_BPS: u16 = 1000;

/// Decimal precision for TWAP results
pub const TWAP_PRECISION: u8 = 6;

/// Min safe trading size (0.00001) to calculate a price. This value considers
/// amount in decimal form with respective token precision.
pub const MIN_TRADE_SIZE: Decimal256 = Decimal256::raw(10000000000000);

/// This structure describes the parameters used for creating a contract.
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
pub struct FlashSwapHookMsg {
    /// This vec contains options for repaying the loan. Each coin in the vec contains the amount
    /// and denom of a token that should be sent back to the pool. The user should send one of the
    /// coins in the vec back to the pool to repay the loan, but not more than one.
    pub required_payment: Vec<Coin>,
    /// An optional binary encoded message passed to the calling contract.
    pub msg: Option<Binary>,
}

#[cw_serde]
pub enum SlippageControl {
    /// The minimum amount of each token to receive from the action.
    /// If this action is `ProvideLiquidity`, the vec should contain exactly one coin with the denom
    /// being the LP token denom.
    /// If this action is `WithdrawLiquidity`, the vec should contain amounts of each of the tokens
    /// you wish to receive from the pool.
    /// If this action is `SwapExactIn`, the vec should contain exactly one coin with the denom
    /// being the asset you wish to receive from the swap.
    /// If this action is `SwapExactOut`, this form of slippage control is not supported and will
    /// error.
    MinOut(Vec<Coin>),
    /// The maximum amount of each token to spend on the action.
    /// If this action is `ProvideLiquidity`, the vec should contain amounts of each of the tokens
    /// you wish to provide as liquidity on the pool.
    /// If this action is `SwapExactOut`, the vec should contain amounts of each of the tokens
    /// you wish to spend on the swap.
    /// If this action is `SwapExactIn` or `WithdrawLiquidity`, this form of slippage control is not
    /// supported and will error.
    MaxIn(Vec<Coin>),
    /// Protects the user from slippage by ensuring that the price of the pool does not move too much.
    /// This form of slippage control is supported for all actions.
    Tolerance {
        /// The user's belief of the price of the pool before the action.
        belief_price: Decimal,
        /// The maximum amount of slippage that is allowed. If the price of the pool after the
        /// action is more than `slippage_tolerance` different from the price supplied in the
        /// `belief_price` field, the transaction will revert.
        slippage_tolerance: Decimal,
    },
}

pub enum ProvideLiquiditySlippageControl {
    MinOutLpAmount(Uint128),
    Tolerance {
        belief_price: Decimal,
        slippage_tolerance: Decimal,
    },
}

pub enum WithdrawLiquiditySlippageControl {
    MinOut(Vec<Coin>),
    Tolerance {
        belief_price: Decimal,
        slippage_tolerance: Decimal,
    },
}

pub enum SwapExactInSlippageControl {
    MinOut(Uint128),
    Tolerance {
        belief_price: Decimal,
        slippage_tolerance: Decimal,
    },
}

pub enum SwapExactOutSlippageControl {
    MaxIn(Vec<Coin>),
    Tolerance {
        belief_price: Decimal,
        slippage_tolerance: Decimal,
    },
}

impl SlippageControl {
    pub fn assert_max_slippage(
        &self,
        old_price: Decimal,
        new_price: Decimal,
        tokens_consumed: Vec<Coin>,
        tokens_sent: Vec<Coin>,
    ) {
        match self {
            SlippageControl::MinOut(min_out_coins) => {
                for x in min_out_coins {
                    let sent = tokens_sent
                        .clone()
                        .into_iter()
                        .find(|c| c.denom == x.denom)
                        .unwrap_or_else(|| coin(0u128, &x.denom));
                    if sent.amount < x.amount {
                        panic!(
                            "Token {} sent to the user is less than the minimum required",
                            x.denom.to_string()
                        );
                    }
                }
            }
            SlippageControl::MaxIn(max_in_coins) => {
                for x in max_in_coins {
                    let consumed = tokens_consumed
                        .clone()
                        .into_iter()
                        .find(|c| c.denom == x.denom)
                        .unwrap_or_else(|| coin(0u128, &x.denom));
                    if consumed.amount > x.amount {
                        panic!(
                            "Token {} consumed from the user is more than the maximum allowed",
                            x.denom.to_string()
                        );
                    }
                }
            }
            SlippageControl::Tolerance {
                slippage_tolerance,
                belief_price,
            } => {
                let slippage = new_price / old_price - Decimal::one();
                if slippage > *slippage_tolerance {
                    panic!("Slippage tolerance not met: slippage > slippage_tolerance");
                }
                let belief_slippage = new_price / *belief_price - Decimal::one();
                if belief_slippage > *slippage_tolerance {
                    panic!("Slippage tolerance not met: belief_slippage > slippage_tolerance");
                }
            }
        }
    }
}

/// This structure describes the execute messages available in the contract.
#[cw_ownable_execute]
#[cw_serde]
pub enum ExecuteMsg<U> {
    /// Provides liquidity to the pool with the native tokens sent to the contract.
    /// Only those tokens that are already in the pool can be provided. If any additional tokens
    /// are sent, the transaction will revert.
    ProvideLiquidity {
        /// The slippage tolerance that allows liquidity provision only if the price in the pool doesn't move too much
        slippage_tolerance: Option<Decimal>,
        /// Determines whether the LP tokens minted for the user is auto_staked in the Generator contract
        auto_stake: Option<bool>,
        /// The recipient of the minted LP tokens
        recipient: Option<String>,

        /// Parameters for slippage control
        slippage_control: SlippageControl,
    },

    /// Withdraws liquidity from the pool. LP tokens should be sent along with the message to the contract.
    WithdrawLiquidity {
        /// The minimum amount of each asset to receive from the pool. If the amount received is
        /// less than this, the transaction will revert.
        min_out: Vec<Coin>,

        /// Parameters for slippage control
        slippage_control: SlippageControl,
    },

    /// Loans the requested tokens from the pool to the calling contract. The tokens will be sent
    /// to the calling contract's address as part of a contract execution with ExecuteMsg
    /// `FlashLoanReceive(FlashSwapHookMsg)`.
    FlashLoan {
        /// The asset to receive as a loan
        receive: Coin,
        /// An optional binary encoded message to be sent back to the calling contract. This will be
        /// included wrapped inside of the `FlashSwapHookMsg` that is sent back to the calling contract.
        msg: Option<Binary>,
    },

    /// Swaps all the native tokens sent to the contract for the asset specified with the `ask_denom` field.
    SwapExactIn {
        /// The asset to receive from the swap
        ask_denom: String,
        /// The minimum amount of `ask_denom` to receive from the swap. If the amount received is
        /// less than this, the transaction will revert.
        min_out: Option<Uint128>,
        /// The address to send the swapped tokens to. If not specified, the tokens will be sent to the caller.
        recipient: Option<String>,
        /// The assets to swap for the asset specified in the `ask_denom` field. If not specified,
        /// the native tokens sent to the contract will be swapped. This is only required if the
        /// `callback` field is supplied, to enable flashswapping.
        offer_assets: Option<Vec<Coin>>,
        /// A binary encoded CosmosMsg used to enable flash swaps. If supplied, the funds
        /// received from the swap will be sent along with this message as a response to the swap
        /// without validating that the required offer assets have been supplied. The offer assets
        /// must instead be sent to the pool at some point before the callback message has finished
        /// executing. The supplied message will be wrapped in a `FlashSwapHookMsg` message.
        callback: Option<Binary>,
    },

    /// Swaps some amount of the sent native tokens for exactly the amount and denom specified in the `ask` field.
    /// Any remaining unused tokens will be sent back to the sender.
    SwapExactOut {
        /// The asset to receive from the swap
        ask: Coin,
        /// The maximum amount of native tokens to offer for the swap. If the amount needed to
        /// receive the requested asset is greater than this, the transaction will revert.
        max_in: Option<Vec<Coin>>,
        /// The address to send the swapped tokens to. If not specified, the tokens will be sent to the caller.
        recipient: Option<String>,
        /// A binary encoded CosmosMsg used to enable flash swaps. If supplied, the funds
        /// received from the swap will be sent along with this message as a response to the swap
        /// without validating that the required offer assets have been supplied. The offer assets
        /// must instead be sent to the pool at some point before the callback message has finished
        /// executing. The supplied message will be wrapped in a `FlashSwapHookMsg` message.
        callback: Option<Binary>,
        /// The assets to swap for the asset specified in the `ask` field. If not specified,
        /// the native tokens sent to the contract will be swapped. This is only required if the
        /// `callback` field is supplied, to enable flashswapping.
        offer_denom: Option<String>,
    },

    /// Update the pool configuration
    UpdateConfig { updates: U },
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

/// This structure describes the query messages available in the contract.
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg<T = Empty, P = Empty> {
    /// Returns information about the pool in an object of type [`PoolInfoResponse`].
    #[returns(PoolInfoResponse)]
    PoolInfo {},

    /// Returns information about a pool in an object of type [`PoolResponse`].
    #[returns(PoolSateResponse)]
    PoolState {},

    /// Returns contract configuration settings in a custom [`ConfigResponse`] structure.
    #[returns(ConfigResponse)]
    Config {},

    /// Simulates withdrawing liquidity from the pool and returns the amount of assets that would be received.
    #[returns(Vec<Coin>)]
    SimulateWithdrawLiquidity { amount: Uint128 },

    /// Simulates a swap with exact amounts of offer assets and returns a `SimulateSwapResponse` object.
    #[returns(SimulateSwapResponse<P>)]
    SimulateSwapExactIn {
        /// The asset to receive from the swap
        ask_denom: String,
        /// The assets to swap for the asset specified in the `ask_denom` field. If not specified,
        /// the native tokens sent to the contract will be swapped. This is only required if the
        /// `callback` field is supplied, to enable flashswapping.
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
    },

    /// Query price from observations
    #[returns(OracleObservation)]
    Observe { seconds_ago: u64 },

    /// Returns the reserves that were in the pool prior to the given block height
    #[returns(Vec<Coin>)]
    PoolReservesAtHeight { block_height: Uint64 },

    /// Queries specific to the pool type
    #[returns(Empty)]
    PoolTypeQueries(T),

    #[returns(Empty)]
    #[serde(skip_serializing)]
    _PhantomData(std::marker::PhantomData<P>),
}

/// This struct is used to return a query result with the total amount of LP tokens and assets in a specific pool.
#[cw_serde]
pub struct PoolSateResponse {
    /// The assets in the pool together with asset amounts
    pub pool_reserves: Vec<Coin>,
    /// The total amount of LP tokens currently issued
    pub lp_token_supply: Uint128,
}

/// This struct is used to return a query result with the general contract configuration.
#[cw_serde]
pub struct ConfigResponse<T = Empty> {
    /// Last timestamp when the cumulative prices in the pool were updated
    pub block_time_last: u64,
    /// The pool's parameters
    pub params: Option<T>,
    /// The contract owner
    pub owner: Addr,
    /// The factory contract address
    pub factory_addr: Addr,
}

/// Holds the configuration for fee sharing
#[cw_serde]
pub struct FeeShareConfig {
    /// The fee shared with the address
    pub bps: u16,
    /// The share is sent to this address on every swap
    pub recipient: Addr,
}

/// This structure holds the parameters that are returned from a swap simulation response
#[cw_serde]
pub struct SimulateSwapResponse<T> {
    /// The amount of offer assets sent to the swap
    pub offer_amount: Uint128,
    /// The amount of ask assets returned by the swap
    pub return_amount: Uint128,
    /// The change in spot price caused by the swap, in percentage form
    pub price_impact: Decimal,
    /// The amount of fees charged by the transaction
    pub commission_amount: Coin,
    /// The difference in percentage between the prior spot price and the execution price
    pub slippage: Decimal,
    /// The pool reserves after the swap
    pub reserves_after: Vec<Coin>,
    /// The parameters unique to the current pool type after the swap
    pub parameters_after: Option<T>,
}

/// This structure describes a migration message.
/// We currently take no arguments for migrations.
#[cw_serde]
pub struct MigrateMsg {}

/// This structure holds XYK pool parameters.
#[cw_serde]
pub struct XYKPoolParams {
    /// Whether asset balances are tracked over blocks or not.
    /// They will not be tracked if the parameter is ignored.
    /// It can not be disabled later once enabled.
    pub track_asset_balances: Option<bool>,
}

/// This structure stores a XYK pool's configuration.
#[cw_serde]
pub struct XYKPoolConfig {
    /// Whether asset balances are tracked over blocks or not.
    pub track_asset_balances: bool,
    // The config for swap fee sharing
    pub fee_share: Option<FeeShareConfig>,
}

/// This enum stores the option available to enable asset balances tracking over blocks.
#[cw_serde]
pub enum XYKPoolUpdateParams {
    /// Enables asset balances tracking over blocks.
    EnableAssetBalancesTracking,
    /// Enables the sharing of swap fees with an external party.
    EnableFeeShare {
        /// The fee shared with the fee_share_address
        fee_share_bps: u16,
        /// The fee_share_bps is sent to this address on every swap
        fee_share_address: String,
    },
    DisableFeeShare,
}

/// This structure holds stableswap pool parameters.
#[cw_serde]
pub struct StablePoolParams {
    /// The current stableswap pool amplification
    pub amp: u64,
    /// The contract owner
    pub owner: Option<String>,
}

/// This structure stores a stableswap pool's configuration.
#[cw_serde]
pub struct StablePoolConfig {
    /// The stableswap pool amplification
    pub amp: Decimal,
    // The config for swap fee sharing
    pub fee_share: Option<FeeShareConfig>,
}

/// This enum stores the options available to start and stop changing a stableswap pool's amplification.
#[cw_serde]
pub enum StablePoolUpdateParams {
    StartChangingAmp {
        next_amp: u64,
        next_amp_time: u64,
    },
    StopChangingAmp {},
    /// Enables the sharing of swap fees with an external party.
    EnableFeeShare {
        /// The fee shared with the fee_share_address
        fee_share_bps: u16,
        /// The fee_share_bps is sent to this address on every swap
        fee_share_address: String,
    },
    DisableFeeShare,
}
