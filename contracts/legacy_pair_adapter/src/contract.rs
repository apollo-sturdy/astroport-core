use std::str::FromStr;
use std::vec;

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    attr, coin, from_binary, to_binary, Addr, BankMsg, Binary, Coin, CosmosMsg, Decimal, Deps,
    DepsMut, Empty, Env, MessageInfo, QuerierWrapper, Reply, ReplyOn, Response, StdError,
    StdResult, SubMsg, SubMsgResponse, SubMsgResult, Uint128, WasmMsg,
};
use cw2::{get_contract_version, set_contract_version};
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg, MinterResponse};

use astroport::asset::{
    addr_opt_validate, format_lp_token_name, Asset, AssetInfo, CoinsExt, PairInfo,
};
use astroport::factory::ExecuteMsg as FactoryExecuteMsg;
use astroport::generator::Cw20HookMsg as GeneratorHookMsg;
use astroport::legacy_pair_wrapper::InstantiateMsg;
use astroport::pair::{ConfigResponse, ExecuteMsg, DEFAULT_SLIPPAGE};
use astroport::pair::{
    Cw20HookMsg, MigrateMsg, PoolResponse, QueryMsg, ReverseSimulationResponse, SimulationResponse,
};
use astroport::pool_new::{
    ExecuteMsg as PoolExecuteMsg, PoolSateResponse, Price, QueryMsg as PoolQueryMsg,
    SimulateSwapResponse, SlippageControl,
};
use astroport::querier::{query_factory_config, query_pair_info};
use astroport::token::InstantiateMsg as TokenInstantiateMsg;
use cw_utils::parse_instantiate_response_data;

use crate::error::ContractError;
use crate::state::{Config, CONFIG, UNDERLYING_LP_TOKEN_DENOM, UNDERLYING_POOL};

/// Contract name that is used for migration.
const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
/// Contract version that is used for migration.
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// A `reply` call code ID used for sub-messages.
const INSTANTIATE_TOKEN_REPLY_ID: u64 = 1;
/// A `reply` call code ID used for sub-messages.
const CREATE_UNDERLYING_POOL_REPLY_ID: u64 = 2;

/// Creates a new contract with the specified parameters in the [`InstantiateMsg`].
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    if msg.asset_infos.len() != 2 {
        return Err(StdError::generic_err("asset_infos must contain exactly two elements").into());
    }

    msg.asset_infos[0].check(deps.api)?;
    msg.asset_infos[1].check(deps.api)?;

    if msg.asset_infos[0] == msg.asset_infos[1] {
        return Err(ContractError::DoublingAssets {});
    }

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let config = Config {
        pair_info: PairInfo {
            contract_addr: env.contract.address.clone(),
            liquidity_token: Addr::unchecked(""),
            asset_infos: msg.asset_infos.clone(),
            pair_type: msg.pair_type.clone(),
        },
        factory_addr: deps.api.addr_validate(msg.factory_addr.as_str())?,
        cw20_adapter_addr: deps.api.addr_validate(&msg.cw20_adapter_addr)?,
    };

    CONFIG.save(deps.storage, &config)?;

    let token_name = format_lp_token_name(&msg.asset_infos, &deps.querier)?;

    // Create the LP token contract
    let sub_msg: Vec<SubMsg> = vec![SubMsg {
        msg: WasmMsg::Instantiate {
            code_id: msg.token_code_id,
            msg: to_binary(&TokenInstantiateMsg {
                name: token_name,
                symbol: "uLP".to_string(),
                decimals: 6,
                initial_balances: vec![],
                mint: Some(MinterResponse {
                    minter: env.contract.address.to_string(),
                    cap: None,
                }),
                marketing: None,
            })?,
            funds: vec![],
            admin: None,
            label: String::from("Astroport LP token"),
        }
        .into(),
        id: INSTANTIATE_TOKEN_REPLY_ID,
        gas_limit: None,
        reply_on: ReplyOn::Success,
    }];

    // Map cw20 assets to their native token denoms
    let asset_infos: Vec<_> = msg
        .asset_infos
        .iter()
        .map(|info| {
            if info.is_native_token() {
                info.clone()
            } else {
                AssetInfo::NativeToken {
                    denom: format!("factory/{}/{}", msg.cw20_adapter_addr, info.to_string()),
                }
            }
        })
        .collect();

    // // Query factory to see if underlying pool already exists
    let pair_info = query_pair_info(&deps.querier, &config.factory_addr, &asset_infos);
    let pair_exists = pair_info.is_ok();

    let mut res = Response::new().add_submessages(sub_msg);

    if !pair_exists {
        // Create the underlying pool
        let create_pool_msg: SubMsg = SubMsg {
            msg: WasmMsg::Execute {
                contract_addr: config.factory_addr.to_string(),
                msg: to_binary(&FactoryExecuteMsg::CreatePair {
                    pair_type: msg.pair_type.clone(),
                    asset_infos,
                    init_params: msg.init_params,
                })?,
                funds: vec![],
            }
            .into(),
            id: CREATE_UNDERLYING_POOL_REPLY_ID,
            gas_limit: None,
            reply_on: ReplyOn::Success,
        };
        res = res.add_submessage(create_pool_msg);
    } else {
        // Store underlying pool address
        let pair_info = pair_info?;
        UNDERLYING_POOL.save(deps.storage, &pair_info.contract_addr)?;
        // TODO: Factory must support native LP token denom
        UNDERLYING_LP_TOKEN_DENOM.save(deps.storage, &pair_info.liquidity_token.to_string())?;
    }

    Ok(res.add_attributes(vec![
        attr("action", "instantiate"),
        attr("pair_type", format!("{}", msg.pair_type)),
        attr(
            "asset_infos",
            format!("{}, {}", msg.asset_infos[0], msg.asset_infos[1]),
        ),
    ]))
}

/// The entry point to the contract for processing replies from submessages.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    match msg {
        Reply {
            id: INSTANTIATE_TOKEN_REPLY_ID,
            result:
                SubMsgResult::Ok(SubMsgResponse {
                    data: Some(data), ..
                }),
        } => {
            let mut config: Config = CONFIG.load(deps.storage)?;

            if config.pair_info.liquidity_token != Addr::unchecked("") {
                return Err(ContractError::Unauthorized {});
            }

            let init_response = parse_instantiate_response_data(data.as_slice())
                .map_err(|e| StdError::generic_err(format!("{e}")))?;

            config.pair_info.liquidity_token =
                deps.api.addr_validate(&init_response.contract_address)?;

            CONFIG.save(deps.storage, &config)?;

            Ok(Response::new()
                .add_attribute("liquidity_token_addr", config.pair_info.liquidity_token))
        }
        Reply {
            id: CREATE_UNDERLYING_POOL_REPLY_ID,
            result:
                SubMsgResult::Ok(SubMsgResponse {
                    data: Some(_data), ..
                }),
        } => {
            let config = CONFIG.load(deps.storage)?;

            // Map cw20 assets to their native token denoms
            let asset_infos: Vec<_> = config
                .pair_info
                .asset_infos
                .iter()
                .map(|info| {
                    if info.is_native_token() {
                        info.clone()
                    } else {
                        AssetInfo::NativeToken {
                            denom: format!(
                                "factory/{}/{}",
                                config.cw20_adapter_addr,
                                info.to_string()
                            ),
                        }
                    }
                })
                .collect();

            // Query pool contract address from factory
            let factory_addr = config.factory_addr;
            let pair_info = query_pair_info(&deps.querier, &factory_addr, &asset_infos)?;

            // Store underlying pool address
            UNDERLYING_POOL.save(deps.storage, &pair_info.contract_addr)?;
            // TODO: Factory must support native LP token denom
            UNDERLYING_LP_TOKEN_DENOM.save(deps.storage, &pair_info.liquidity_token.to_string())?;

            Ok(Response::new()
                .add_attribute("liquidity_token_addr", config.pair_info.liquidity_token))
        }
        _ => Err(ContractError::FailedToParseReply {}),
    }
}

/// Exposes all the execute functions available in the contract.
///
/// ## Variants
/// * **ExecuteMsg::UpdateConfig { params: Binary }** Not supported.
///
/// * **ExecuteMsg::Receive(msg)** Receives a message of type [`Cw20ReceiveMsg`] and processes
/// it depending on the received template.
///
/// * **ExecuteMsg::ProvideLiquidity {
///             assets,
///             slippage_tolerance,
///             auto_stake,
///             receiver,
///         }** Provides liquidity in the pair with the specified input parameters.
///
/// * **ExecuteMsg::Swap {
///             offer_asset,
///             belief_price,
///             max_spread,
///             to,
///         }** Performs a swap operation with the specified parameters.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Receive(msg) => receive_cw20(deps, env, info, msg),
        ExecuteMsg::ProvideLiquidity {
            assets,
            slippage_tolerance,
            auto_stake,
            receiver,
        } => provide_liquidity(
            deps,
            env,
            info,
            assets,
            slippage_tolerance,
            auto_stake,
            receiver,
        ),
        ExecuteMsg::Swap {
            offer_asset,
            belief_price,
            max_spread,
            to,
            ..
        } => {
            offer_asset.info.check(deps.api)?;
            if !offer_asset.is_native_token() {
                return Err(ContractError::Cw20DirectSwap {});
            }

            let to_addr = addr_opt_validate(deps.api, &to)?;

            swap(
                deps,
                env,
                info.clone(),
                info.sender,
                offer_asset,
                belief_price,
                max_spread,
                to_addr,
            )
        }
        _ => Err(ContractError::NonSupported {}),
    }
}

/// Receives a message of type [`Cw20ReceiveMsg`] and processes it depending on the received template.
///
/// * **cw20_msg** is the CW20 message that has to be processed.
pub fn receive_cw20(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    cw20_msg: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    match from_binary(&cw20_msg.msg)? {
        Cw20HookMsg::Swap {
            belief_price,
            max_spread,
            to,
            ..
        } => {
            // Only asset contract can execute this message
            let mut authorized = false;
            let config = CONFIG.load(deps.storage)?;

            for pool in config.pair_info.asset_infos {
                if let AssetInfo::Token { contract_addr, .. } = &pool {
                    if contract_addr == info.sender {
                        authorized = true;
                    }
                }
            }

            if !authorized {
                return Err(ContractError::Unauthorized {});
            }

            let to_addr = addr_opt_validate(deps.api, &to)?;
            let contract_addr = info.sender.clone();

            swap(
                deps,
                env,
                info,
                Addr::unchecked(cw20_msg.sender),
                Asset {
                    info: AssetInfo::Token { contract_addr },
                    amount: cw20_msg.amount,
                },
                belief_price,
                max_spread,
                to_addr,
            )
        }
        Cw20HookMsg::WithdrawLiquidity { assets } => withdraw_liquidity(
            deps,
            env,
            info,
            Addr::unchecked(cw20_msg.sender),
            cw20_msg.amount,
            assets,
        ),
    }
}

/// Provides liquidity in the pair with the specified input parameters.
///
/// * **assets** is an array with assets available in the pool.
///
/// * **slippage_tolerance** is an optional parameter which is used to specify how much
/// the pool price can move until the provide liquidity transaction goes through.
///
/// * **auto_stake** is an optional parameter which determines whether the LP tokens minted after
/// liquidity provision are automatically staked in the Generator contract on behalf of the LP token receiver.
///
/// * **receiver** is an optional parameter which defines the receiver of the LP tokens.
/// If no custom receiver is specified, the pair will mint LP tokens for the function caller.
///
/// NOTE - the address that wants to provide liquidity should approve the pair contract to pull its relevant tokens.
pub fn provide_liquidity(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    assets: Vec<Asset>,
    _slippage_tolerance: Option<Decimal>,
    auto_stake: Option<bool>,
    receiver: Option<String>,
) -> Result<Response, ContractError> {
    if assets.len() != 2 {
        return Err(StdError::generic_err("asset_infos must contain exactly two elements").into());
    }
    assets[0].info.check(deps.api)?;
    assets[1].info.check(deps.api)?;

    let auto_stake = auto_stake.unwrap_or(false);

    let config = CONFIG.load(deps.storage)?;
    info.funds
        .assert_coins_properly_sent(&assets, &config.pair_info.asset_infos)?;
    let asset_infos = &config.pair_info.asset_infos;
    let deposits = [
        assets
            .iter()
            .find(|a| a.info.equal(&asset_infos[0]))
            .map(|a| a.amount)
            .expect("Wrong asset info is given"),
        assets
            .iter()
            .find(|a| a.info.equal(&asset_infos[1]))
            .map(|a| a.amount)
            .expect("Wrong asset info is given"),
    ];

    if deposits[0].is_zero() && deposits[1].is_zero() {
        return Err(ContractError::InvalidZeroAmount {});
    }

    let mut messages = vec![];
    for (i, asset_info) in asset_infos.iter().enumerate() {
        // If the asset is a token contract, then we need to execute a TransferFrom msg to receive assets
        if let AssetInfo::Token { contract_addr, .. } = &asset_info {
            if deposits[i] > Uint128::zero() {
                messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: contract_addr.to_string(),
                    msg: to_binary(&Cw20ExecuteMsg::TransferFrom {
                        owner: info.sender.to_string(),
                        recipient: env.contract.address.to_string(),
                        amount: deposits[i],
                    })?,
                    funds: vec![],
                }));
            }
        }
    }

    // Wrap any Cw20s into native tokens
    for (i, deposit) in deposits.iter().enumerate() {
        if !assets[i].is_native_token() && assets[i].amount > Uint128::zero() {
            messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: assets[i].info.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Send {
                    contract: config.cw20_adapter_addr.to_string(),
                    amount: *deposit,
                    msg: to_binary(&Empty {})?,
                })?,
                funds: vec![],
            }));
        }
    }

    // Provide liquidity in the underlying pool
    let underlying_pool = UNDERLYING_POOL.load(deps.storage)?;
    let funds: Vec<Coin> = assets
        .iter()
        .filter(|x| x.amount > Uint128::zero())
        .map(|x| {
            if x.is_native_token() {
                coin(x.amount.u128(), x.info.to_string())
            } else {
                coin(
                    x.amount.u128(),
                    format!(
                        "factory/{}/{}",
                        config.cw20_adapter_addr,
                        x.info.to_string()
                    ),
                )
            }
        })
        .collect();
    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.pair_info.contract_addr.to_string(),
        msg: to_binary(&PoolExecuteMsg::<Empty>::ProvideLiquidity {
            auto_stake: Some(false),
            min_out: None,
            recipient: None,
            slippage_control: None, // TODO: can we even do slippage control without belief price..?
        })?,
        funds: funds.clone(),
    }));

    // Simulate providing liquidity in the underlying pool
    let minted_lp_tokens: Coin = deps.querier.query_wasm_smart(
        underlying_pool,
        &PoolQueryMsg::<Empty>::SimulateProvideLiquidity {
            assets: funds,
            reserves: None,
            params: None,
        },
    )?;

    // Mint LP tokens for the sender or for the receiver (if set)
    let recipient = addr_opt_validate(deps.api, &receiver)?.unwrap_or_else(|| info.sender.clone());
    let msgs = mint_liquidity_token_message(
        deps.querier,
        &config,
        &env.contract.address,
        &recipient,
        minted_lp_tokens.amount,
        auto_stake,
    )?;
    messages.extend(msgs);

    Ok(Response::new().add_messages(messages).add_attributes(vec![
        attr("action", "provide_liquidity"),
        attr("sender", info.sender),
        attr("receiver", recipient),
        attr("assets", format!("{}, {}", assets[0], assets[1])),
    ]))
}

/// Mint LP tokens for a beneficiary and auto stake the tokens in the Generator contract (if auto staking is specified).
///
/// * **recipient** is the LP token recipient.
///
/// * **amount** is the amount of LP tokens that will be minted for the recipient.
///
/// * **auto_stake** determines whether the newly minted LP tokens will
/// be automatically staked in the Generator on behalf of the recipient.
fn mint_liquidity_token_message(
    querier: QuerierWrapper,
    config: &Config,
    contract_address: &Addr,
    recipient: &Addr,
    amount: Uint128,
    auto_stake: bool,
) -> Result<Vec<CosmosMsg>, ContractError> {
    let lp_token = &config.pair_info.liquidity_token;

    // If no auto-stake - just mint to recipient
    if !auto_stake {
        return Ok(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: lp_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Mint {
                recipient: recipient.to_string(),
                amount,
            })?,
            funds: vec![],
        })]);
    }

    // Mint for the pair contract and stake into the Generator contract
    let generator = query_factory_config(&querier, &config.factory_addr)?.generator_address;

    if let Some(generator) = generator {
        Ok(vec![
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: lp_token.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Mint {
                    recipient: contract_address.to_string(),
                    amount,
                })?,
                funds: vec![],
            }),
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: lp_token.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Send {
                    contract: generator.to_string(),
                    amount,
                    msg: to_binary(&GeneratorHookMsg::DepositFor(recipient.to_string()))?,
                })?,
                funds: vec![],
            }),
        ])
    } else {
        Err(ContractError::AutoStakeError {})
    }
}

/// Withdraw liquidity from the pool.
/// * **sender** is the address that will receive assets back from the pair contract.
///
/// * **amount** is the amount of LP tokens to burn.
pub fn withdraw_liquidity(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    sender: Addr,
    amount: Uint128,
    assets: Vec<Asset>,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage).unwrap();

    if info.sender != config.pair_info.liquidity_token {
        return Err(ContractError::Unauthorized {});
    }

    if !assets.is_empty() {
        return Err(StdError::generic_err("Imbalanced withdraw is currently disabled").into());
    };

    // Burn the CW20 LP tokens
    let mut messages: Vec<CosmosMsg> = vec![CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: config.pair_info.liquidity_token.to_string(),
        msg: to_binary(&Cw20ExecuteMsg::Burn { amount })?,
        funds: vec![],
    })];

    // Withdraw assets in underlying pool
    let underlying_pool = UNDERLYING_POOL.load(deps.storage)?;
    let native_lp_denom = UNDERLYING_LP_TOKEN_DENOM.load(deps.storage)?;
    let withdraw_msg: CosmosMsg = WasmMsg::Execute {
        contract_addr: underlying_pool.to_string(),
        msg: to_binary(&PoolExecuteMsg::<Empty>::WithdrawLiquidity {
            min_out: vec![],
            slippage_control: None,
        })?,
        funds: vec![coin(amount.u128(), native_lp_denom)],
    }
    .into();
    messages.push(withdraw_msg);

    // Simulate withdrawing assets from the underlying pool
    let withdrawn_assets: Vec<Coin> = deps.querier.query_wasm_smart(
        underlying_pool,
        &PoolQueryMsg::<Empty>::SimulateWithdrawLiquidity {
            amount,
            reserves: None,
            params: None,
        },
    )?;

    // Unwrap any wrapped assets and send withdraw assets to user
    for asset in withdrawn_assets {
        if asset
            .denom
            .starts_with(format!("factory/{}/", config.cw20_adapter_addr).as_str())
        {
            messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: config.cw20_adapter_addr.to_string(),
                msg: to_binary(&cw20_adapter::msg::ExecuteMsg::RedeemAndTransfer {
                    recipient: Some(sender.to_string()),
                })?,
                funds: vec![asset],
            }));
        } else {
            messages.push(CosmosMsg::Bank(BankMsg::Send {
                to_address: sender.to_string(),
                amount: vec![asset],
            }));
        }
    }

    Ok(Response::new().add_messages(messages).add_attributes(vec![
        attr("action", "withdraw_liquidity"),
        attr("sender", sender),
        attr("withdrawn_share", amount),
    ]))
}

/// Performs an swap operation with the specified parameters. The trader must approve the
/// pool contract to transfer offer assets from their wallet.
///
/// * **sender** is the sender of the swap operation.
///
/// * **offer_asset** proposed asset for swapping.
///
/// * **belief_price** is used to calculate the maximum swap spread.
///
/// * **max_spread** sets the maximum spread of the swap operation.
///
/// * **to** sets the recipient of the swap operation.
///
/// NOTE - the address that wants to swap should approve the pair contract to pull the offer token.
#[allow(clippy::too_many_arguments)]
pub fn swap(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    sender: Addr,
    offer_asset: Asset,
    belief_price: Option<Decimal>,
    max_spread: Option<Decimal>,
    to: Option<Addr>,
) -> Result<Response, ContractError> {
    offer_asset.assert_sent_native_token_balance(&info)?;

    let config = CONFIG.load(deps.storage)?;
    let asset_infos = config.pair_info.asset_infos;

    let ask_asset_info = if offer_asset.info.equal(&asset_infos[0]) {
        asset_infos[1].clone()
    } else if offer_asset.info.equal(&asset_infos[1]) {
        asset_infos[0].clone()
    } else {
        return Err(ContractError::AssetMismatch {});
    };
    let ask_denom = match &ask_asset_info {
        AssetInfo::NativeToken { denom } => denom.clone(),
        AssetInfo::Token { contract_addr, .. } => {
            format!("factory/{}/{}", config.cw20_adapter_addr, contract_addr)
        }
    };
    let ask_asset_is_cw20 = !ask_asset_info.is_native_token();
    let offer_asset_is_cw20 = !offer_asset.info.is_native_token();
    let wrapped_offer_denom = format!("factory/{}/{}", config.cw20_adapter_addr, info.sender);

    let mut msgs: Vec<CosmosMsg> = vec![];

    // If offer asset is CW20, Wrap it into native token
    let wrap_msg: CosmosMsg = WasmMsg::Execute {
        contract_addr: config.cw20_adapter_addr.to_string(),
        msg: to_binary(&Cw20ExecuteMsg::Send {
            contract: config.cw20_adapter_addr.to_string(),
            amount: offer_asset.amount,
            msg: to_binary(&Empty {})?,
        })?,
        funds: vec![],
    }
    .into();
    if offer_asset_is_cw20 {
        msgs.push(wrap_msg);
    }

    // Determine swap recipient
    // If the ask asset is wrapped, we set recipient to None so we can receive the wrapped asset here and unwrap it.
    // Otherwise set it to either the users specified `to` address, or the user themself.
    let recipient = if ask_asset_is_cw20 {
        None
    } else {
        Some(to.unwrap_or(sender).to_string())
    };

    // Swap in the underlying pool
    let default_spread = Decimal::from_str(DEFAULT_SLIPPAGE)?;
    let max_spread = max_spread.unwrap_or(default_spread);
    let underlying_pool = UNDERLYING_POOL.load(deps.storage)?;
    let swap_msg: CosmosMsg = WasmMsg::Execute {
        contract_addr: underlying_pool.to_string(),
        msg: to_binary(&PoolExecuteMsg::<Empty>::SwapExactIn {
            ask_denom: ask_denom.clone(),
            recipient: recipient.clone(),
            min_out: None,
            slippage_control: belief_price.map(|belief_price| SlippageControl {
                belief_price: Price {
                    base_asset: wrapped_offer_denom.clone(),
                    price: belief_price,
                    quote_asset: ask_denom.clone(),
                },
                slippage_tolerance: max_spread,
            }),
        })?,
        funds: vec![coin(offer_asset.amount.u128(), &wrapped_offer_denom)],
    }
    .into();
    msgs.push(swap_msg);

    // Simulate swap in the underlying pool to see how many ask assets will be received
    let res: SimulateSwapResponse = deps.querier.query_wasm_smart(
        underlying_pool,
        &PoolQueryMsg::<Empty>::SimulateSwapExactIn {
            ask_denom,
            offer_assets: vec![coin(offer_asset.amount.u128(), wrapped_offer_denom)],
            reserves: None,
            params: None,
        },
    )?;

    // If the ask asset is wrapped add message to unwrap it and send to user
    let unwrap_msg: CosmosMsg = WasmMsg::Execute {
        contract_addr: config.cw20_adapter_addr.to_string(),
        msg: to_binary(&cw20_adapter::msg::ExecuteMsg::RedeemAndTransfer { recipient })?,
        funds: vec![res.return_asset],
    }
    .into();
    if ask_asset_is_cw20 {
        msgs.push(unwrap_msg);
    }

    Ok(Response::new().add_messages(msgs))
}

/// Exposes all the queries available in the contract.
///
/// ## Queries
/// * **QueryMsg::Pair {}** Returns information about the pair in an object of type [`PairInfo`].
///
/// * **QueryMsg::Pool {}** Returns information about the amount of assets in the pair contract as
/// well as the amount of LP tokens issued using an object of type [`PoolResponse`].
///
/// * **QueryMsg::Share { amount }** Returns the amount of assets that could be withdrawn from the pool
/// using a specific amount of LP tokens. The result is returned in a vector that contains objects of type [`Asset`].
///
/// * **QueryMsg::Simulation { offer_asset }** Returns the result of a swap simulation using a [`SimulationResponse`] object.
///
/// * **QueryMsg::ReverseSimulation { ask_asset }** Returns the result of a reverse swap simulation  using
/// a [`ReverseSimulationResponse`] object.
///
/// * **QueryMsg::CumulativePrices {}** Returns information about cumulative prices for the assets in the
/// pool using a [`CumulativePricesResponse`] object.
///
/// * **QueryMsg::Config {}** Returns the configuration for the pair contract using a [`ConfigResponse`] object.
///
/// * **QueryMsg::AssetBalanceAt { asset_info, block_height }** Returns the balance of the specified asset that was in the pool
/// just preceeding the moment of the specified block height creation.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Pair {} => to_binary(&CONFIG.load(deps.storage)?.pair_info),
        QueryMsg::Pool {} => to_binary(&query_pool(deps)?),
        QueryMsg::Share { amount } => to_binary(&query_share(deps, amount)?),
        QueryMsg::Simulation { offer_asset, .. } => {
            to_binary(&query_simulation(deps, offer_asset)?)
        }
        QueryMsg::ReverseSimulation { ask_asset, .. } => {
            to_binary(&query_reverse_simulation(deps, ask_asset)?)
        }
        QueryMsg::CumulativePrices {} => unimplemented!("Cumulative prices are not implemented"),
        QueryMsg::Config {} => to_binary(&query_config(deps)?),
        QueryMsg::AssetBalanceAt {
            asset_info: _,
            block_height: _,
        } => unimplemented!("Asset balance at is not implemented"),
        _ => Err(StdError::generic_err("Query is not supported")),
    }
}

/// Returns the amounts of assets in the pair contract as well as the amount of LP
/// tokens currently minted in an object of type [`PoolResponse`].
pub fn query_pool(deps: Deps) -> StdResult<PoolResponse> {
    let config = CONFIG.load(deps.storage)?;

    // Query underlying pool
    let underlying_pool = UNDERLYING_POOL.load(deps.storage)?;
    let res: PoolSateResponse = deps
        .querier
        .query_wasm_smart(underlying_pool, &PoolQueryMsg::<Empty>::PoolState {})?;

    let reserves: Vec<Asset> = res
        .pool_reserves
        .into_iter()
        .map(|x| {
            if x.denom
                .starts_with(format!("factory/{}/", config.cw20_adapter_addr).as_str())
            {
                Asset {
                    info: AssetInfo::Token {
                        contract_addr: Addr::unchecked(x.denom.split("/").last().unwrap()),
                    },
                    amount: Uint128::from(x.amount),
                }
            } else {
                Asset {
                    info: AssetInfo::NativeToken {
                        denom: x.denom.to_string(),
                    },
                    amount: Uint128::from(x.amount),
                }
            }
        })
        .collect();

    let resp: PoolResponse = PoolResponse {
        assets: reserves,
        total_share: res.lp_token_supply.amount,
    };

    Ok(resp)
}

/// Returns the amount of assets that could be withdrawn from the pool using a specific amount of LP tokens.
/// The result is returned in a vector that contains objects of type [`Asset`].
///
/// * **amount** is the amount of LP tokens for which we calculate associated amounts of assets.
pub fn query_share(deps: Deps, amount: Uint128) -> StdResult<Vec<Asset>> {
    let config = CONFIG.load(deps.storage)?;

    // Simulate withdrawing assets from the underlying pool
    let underlying_pool = UNDERLYING_POOL.load(deps.storage)?;
    let withdrawn_assets: Vec<Coin> = deps.querier.query_wasm_smart(
        underlying_pool,
        &PoolQueryMsg::<Empty>::SimulateWithdrawLiquidity {
            amount,
            reserves: None,
            params: None,
        },
    )?;

    // Unwrap any wrapped assets
    Ok(withdrawn_assets
        .into_iter()
        .map(|x| {
            if x.denom
                .starts_with(format!("factory/{}/", config.cw20_adapter_addr).as_str())
            {
                Asset {
                    info: AssetInfo::Token {
                        contract_addr: Addr::unchecked(x.denom.split("/").last().unwrap()),
                    },
                    amount: Uint128::from(x.amount),
                }
            } else {
                Asset {
                    info: AssetInfo::NativeToken {
                        denom: x.denom.to_string(),
                    },
                    amount: Uint128::from(x.amount),
                }
            }
        })
        .collect())
}

/// Returns information about a swap simulation in a [`SimulationResponse`] object.
///
/// * **offer_asset** is the asset to swap as well as an amount of the said asset.
pub fn query_simulation(deps: Deps, offer_asset: Asset) -> StdResult<SimulationResponse> {
    let config = CONFIG.load(deps.storage)?;

    let asset_infos = config.pair_info.asset_infos;

    let ask_asset_info = if offer_asset.info.equal(&asset_infos[0]) {
        asset_infos[1].clone()
    } else if offer_asset.info.equal(&asset_infos[1]) {
        asset_infos[0].clone()
    } else {
        return Err(StdError::generic_err(
            "Given offer asset does not belong in the pair",
        ));
    };
    let ask_denom = match ask_asset_info {
        AssetInfo::NativeToken { denom } => denom,
        AssetInfo::Token { contract_addr, .. } => {
            format!("factory/{}/{}", config.cw20_adapter_addr, contract_addr)
        }
    };
    let native_offer_denom = match offer_asset.info {
        AssetInfo::NativeToken { denom } => denom,
        AssetInfo::Token { contract_addr, .. } => {
            format!("factory/{}/{}", config.cw20_adapter_addr, contract_addr)
        }
    };

    // Simulate on underlying pool
    let underlying_pool = UNDERLYING_POOL.load(deps.storage)?;
    let res: SimulateSwapResponse = deps.querier.query_wasm_smart(
        underlying_pool,
        &PoolQueryMsg::<Empty>::SimulateSwapExactIn {
            ask_denom,
            offer_assets: vec![coin(offer_asset.amount.u128(), native_offer_denom)],
            reserves: None,
            params: None,
        },
    )?;

    Ok(SimulationResponse {
        return_amount: res.return_asset.amount,
        spread_amount: res.slippage * res.return_asset.amount,
        commission_amount: res.commission.amount,
    })
}

/// Returns information about a reverse swap simulation in a [`ReverseSimulationResponse`] object.
///
/// * **ask_asset** is the asset to swap to as well as the desired amount of ask
/// assets to receive from the swap.
pub fn query_reverse_simulation(
    deps: Deps,
    ask_asset: Asset,
) -> StdResult<ReverseSimulationResponse> {
    let config = CONFIG.load(deps.storage)?;

    let asset_infos = config.pair_info.asset_infos;
    let offer_asset_info = if ask_asset.info.equal(&asset_infos[0]) {
        asset_infos[1].clone()
    } else if ask_asset.info.equal(&asset_infos[1]) {
        asset_infos[0].clone()
    } else {
        return Err(StdError::generic_err(
            "Given ask asset does not belong in the pair",
        ));
    };

    let ask_denom = match ask_asset.info {
        AssetInfo::NativeToken { denom } => denom,
        AssetInfo::Token { contract_addr, .. } => {
            format!("factory/{}/{}", config.cw20_adapter_addr, contract_addr)
        }
    };
    let offer_denom = match offer_asset_info {
        AssetInfo::NativeToken { denom } => denom,
        AssetInfo::Token { contract_addr, .. } => {
            format!("factory/{}/{}", config.cw20_adapter_addr, contract_addr)
        }
    };

    // Simulate on underlying pool
    let underlying_pool = UNDERLYING_POOL.load(deps.storage)?;
    let res: SimulateSwapResponse = deps.querier.query_wasm_smart(
        underlying_pool,
        &PoolQueryMsg::<Empty>::SimulateSwapExactOut {
            ask: coin(ask_asset.amount.u128(), ask_denom),
            offer_denom,
            reserves: None,
            params: None,
        },
    )?;

    Ok(ReverseSimulationResponse {
        offer_amount: res.offer_assets[0].amount,
        spread_amount: res.slippage * res.offer_assets[0].amount,
        commission_amount: res.commission.amount,
    })
}

/// Returns the pair contract configuration in a [`ConfigResponse`] object.
pub fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config: Config = CONFIG.load(deps.storage)?;

    let factory_config = query_factory_config(&deps.querier, &config.factory_addr)?;

    Ok(ConfigResponse {
        block_time_last: 0,
        params: None,
        owner: factory_config.owner,
        factory_addr: config.factory_addr,
    })
}

/// Manages the contract migration.
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    let contract_version = get_contract_version(deps.storage)?;

    if contract_version.contract != CONTRACT_NAME {
        return Err(ContractError::MigrationError {});
    }
    if contract_version.version == CONTRACT_VERSION {
        return Err(ContractError::MigrationError {});
    }

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    Ok(Response::default().add_attributes([
        ("previous_contract_name", contract_version.contract.as_str()),
        (
            "previous_contract_version",
            contract_version.version.as_str(),
        ),
        ("new_contract_name", CONTRACT_NAME),
        ("new_contract_version", CONTRACT_VERSION),
    ]))
}
