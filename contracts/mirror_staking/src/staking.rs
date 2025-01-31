use cosmwasm_std::{
    attr, to_binary, Addr, CanonicalAddr, Coin, CosmosMsg, Decimal, DepsMut, Env, MessageInfo,
    Response, StdError, StdResult, Storage, Uint128, WasmMsg,
};

use crate::rewards::before_share_change;
use crate::state::{
    read_config, read_is_migrated, read_pool_info, rewards_read, rewards_store, store_is_migrated,
    store_pool_info, Config, PoolInfo, RewardInfo,
};

use cw20::Cw20ExecuteMsg;
use mirror_protocol::staking::ExecuteMsg;
use daodiseoswap::asset::{Asset, AssetInfo, PairInfo};
use daodiseoswap::pair::ExecuteMsg as PairExecuteMsg;
use daodiseoswap::querier::{query_pair_info, query_token_balance};

pub fn bond(
    deps: DepsMut,
    staker_addr: Addr,
    asset_token: Addr,
    amount: Uint128,
) -> StdResult<Response> {
    let staker_addr_raw: CanonicalAddr = deps.api.addr_canonicalize(staker_addr.as_str())?;
    let asset_token_raw: CanonicalAddr = deps.api.addr_canonicalize(asset_token.as_str())?;
    _increase_bond_amount(
        deps.storage,
        &staker_addr_raw,
        &asset_token_raw,
        amount,
        false,
    )?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "bond"),
        attr("staker_addr", staker_addr.as_str()),
        attr("asset_token", asset_token.as_str()),
        attr("amount", amount.to_string()),
    ]))
}

pub fn unbond(
    deps: DepsMut,
    staker_addr: Addr,
    asset_token: Addr,
    amount: Uint128,
) -> StdResult<Response> {
    let staker_addr_raw: CanonicalAddr = deps.api.addr_canonicalize(staker_addr.as_str())?;
    let asset_token_raw: CanonicalAddr = deps.api.addr_canonicalize(asset_token.as_str())?;
    let staking_token: CanonicalAddr = _decrease_bond_amount(
        deps.storage,
        &staker_addr_raw,
        &asset_token_raw,
        amount,
        false,
    )?;
    let staking_token_addr: Addr = deps.api.addr_humanize(&staking_token)?;

    Ok(Response::new()
        .add_message(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.addr_humanize(&staking_token)?.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: staker_addr.to_string(),
                amount,
            })?,
            funds: vec![],
        }))
        .add_attributes(vec![
            attr("action", "unbond"),
            attr("staker_addr", staker_addr.as_str()),
            attr("asset_token", asset_token.as_str()),
            attr("amount", amount.to_string()),
            attr("staking_token", staking_token_addr.as_str()),
        ]))
}

// only mint contract can execute the operation
pub fn increase_short_token(
    deps: DepsMut,
    info: MessageInfo,
    staker_addr: Addr,
    asset_token: Addr,
    amount: Uint128,
) -> StdResult<Response> {
    let config: Config = read_config(deps.storage)?;
    if deps.api.addr_canonicalize(info.sender.as_str())? != config.mint_contract {
        return Err(StdError::generic_err("unauthorized"));
    }

    let staker_addr_raw: CanonicalAddr = deps.api.addr_canonicalize(staker_addr.as_str())?;
    let asset_token_raw: CanonicalAddr = deps.api.addr_canonicalize(asset_token.as_str())?;

    _increase_bond_amount(
        deps.storage,
        &staker_addr_raw,
        &asset_token_raw,
        amount,
        true,
    )?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "increase_short_token"),
        attr("staker_addr", staker_addr.as_str()),
        attr("asset_token", asset_token.as_str()),
        attr("amount", amount.to_string()),
    ]))
}

// only mint contract can execute the operation
pub fn decrease_short_token(
    deps: DepsMut,
    info: MessageInfo,
    staker_addr: Addr,
    asset_token: Addr,
    amount: Uint128,
) -> StdResult<Response> {
    let config: Config = read_config(deps.storage)?;
    if deps.api.addr_canonicalize(info.sender.as_str())? != config.mint_contract {
        return Err(StdError::generic_err("unauthorized"));
    }

    let staker_addr_raw: CanonicalAddr = deps.api.addr_canonicalize(staker_addr.as_str())?;
    let asset_token_raw: CanonicalAddr = deps.api.addr_canonicalize(asset_token.as_str())?;

    // not used
    let _ = _decrease_bond_amount(
        deps.storage,
        &staker_addr_raw,
        &asset_token_raw,
        amount,
        true,
    )?;

    Ok(Response::new().add_attributes(vec![
        attr("action", "decrease_short_token"),
        attr("staker_addr", staker_addr.as_str()),
        attr("asset_token", asset_token.as_str()),
        attr("amount", amount.to_string()),
    ]))
}

pub fn auto_stake(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    assets: [Asset; 2],
    slippage_tolerance: Option<Decimal>,
) -> StdResult<Response> {
    let config: Config = read_config(deps.storage)?;
    let daodiseoswap_factory: Addr = deps.api.addr_humanize(&config.daodiseoswap_factory)?;

    let mut native_asset_op: Option<Asset> = None;
    let mut token_info_op: Option<(Addr, Uint128)> = None;
    for asset in assets.iter() {
        match asset.info.clone() {
            AssetInfo::NativeToken { .. } => {
                asset.assert_sent_native_token_balance(&info)?;
                native_asset_op = Some(asset.clone())
            }
            AssetInfo::Token { contract_addr } => {
                token_info_op = Some((deps.api.addr_validate(&contract_addr)?, asset.amount))
            }
        }
    }

    // will fail if one of them is missing
    let native_asset: Asset = match native_asset_op {
        Some(v) => v,
        None => return Err(StdError::generic_err("Missing native asset")),
    };
    let (token_addr, token_amount) = match token_info_op {
        Some(v) => v,
        None => return Err(StdError::generic_err("Missing token asset")),
    };

    // query pair info to obtain pair contract address
    let asset_infos: [AssetInfo; 2] = [assets[0].info.clone(), assets[1].info.clone()];
    let daodiseoswap_pair: PairInfo = query_pair_info(&deps.querier, daodiseoswap_factory, &asset_infos)?;

    // assert the token and lp token match with pool info
    let pool_info: PoolInfo = read_pool_info(
        deps.storage,
        &deps.api.addr_canonicalize(token_addr.as_str())?,
    )?;

    if pool_info.staking_token
        != deps
            .api
            .addr_canonicalize(daodiseoswap_pair.liquidity_token.as_str())?
    {
        return Err(StdError::generic_err("Invalid staking token"));
    }

    // get current lp token amount to later compute the recived amount
    let prev_staking_token_amount = query_token_balance(
        &deps.querier,
        deps.api.addr_validate(&daodiseoswap_pair.liquidity_token)?,
        env.contract.address.clone(),
    )?;

    // compute tax
    let tax_amount: Uint128 = native_asset.compute_tax(&deps.querier)?;

    // 1. Transfer token asset to staking contract
    // 2. Increase allowance of token for pair contract
    // 3. Provide liquidity
    // 4. Execute staking hook, will stake in the name of the sender
    Ok(Response::new()
        .add_messages(vec![
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: token_addr.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::TransferFrom {
                    owner: info.sender.to_string(),
                    recipient: env.contract.address.to_string(),
                    amount: token_amount,
                })?,
                funds: vec![],
            }),
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: token_addr.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::IncreaseAllowance {
                    spender: daodiseoswap_pair.contract_addr.to_string(),
                    amount: token_amount,
                    expires: None,
                })?,
                funds: vec![],
            }),
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: daodiseoswap_pair.contract_addr.to_string(),
                msg: to_binary(&PairExecuteMsg::ProvideLiquidity {
                    assets: [
                        Asset {
                            amount: native_asset.amount.checked_sub(tax_amount)?,
                            info: native_asset.info.clone(),
                        },
                        Asset {
                            amount: token_amount,
                            info: AssetInfo::Token {
                                contract_addr: token_addr.to_string(),
                            },
                        },
                    ],
                    slippage_tolerance,
                    receiver: None,
                })?,
                funds: vec![Coin {
                    denom: native_asset.info.to_string(),
                    amount: native_asset.amount.checked_sub(tax_amount)?,
                }],
            }),
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: env.contract.address.to_string(),
                msg: to_binary(&ExecuteMsg::AutoStakeHook {
                    asset_token: token_addr.to_string(),
                    staking_token: daodiseoswap_pair.liquidity_token,
                    staker_addr: info.sender.to_string(),
                    prev_staking_token_amount,
                })?,
                funds: vec![],
            }),
        ])
        .add_attributes(vec![
            attr("action", "auto_stake"),
            attr("asset_token", token_addr.to_string()),
            attr("tax_amount", tax_amount.to_string()),
        ]))
}

pub fn auto_stake_hook(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    asset_token: Addr,
    staking_token: Addr,
    staker_addr: Addr,
    prev_staking_token_amount: Uint128,
) -> StdResult<Response> {
    // only can be called by itself
    if info.sender != env.contract.address {
        return Err(StdError::generic_err("unauthorized"));
    }

    // stake all lp tokens received, compare with staking token amount before liquidity provision was executed
    let current_staking_token_amount =
        query_token_balance(&deps.querier, staking_token, env.contract.address)?;
    let amount_to_stake = current_staking_token_amount.checked_sub(prev_staking_token_amount)?;

    bond(deps, staker_addr, asset_token, amount_to_stake)
}

fn _increase_bond_amount(
    storage: &mut dyn Storage,
    staker_addr: &CanonicalAddr,
    asset_token: &CanonicalAddr,
    amount: Uint128,
    is_short: bool,
) -> StdResult<()> {
    let mut pool_info: PoolInfo = read_pool_info(storage, asset_token)?;
    let mut reward_info: RewardInfo = rewards_read(storage, staker_addr, is_short)
        .load(asset_token.as_slice())
        .unwrap_or_else(|_| RewardInfo {
            index: Decimal::zero(),
            bond_amount: Uint128::zero(),
            pending_reward: Uint128::zero(),
        });

    // check if the position should be migrated
    let is_position_migrated = read_is_migrated(storage, asset_token, staker_addr);
    if !is_short && pool_info.migration_params.is_some() {
        // the pool has been migrated, if position is not migrated and has tokens bonded, return error
        if !reward_info.bond_amount.is_zero() && !is_position_migrated {
            return Err(StdError::generic_err("The LP token for this asset has been deprecated, withdraw all your deprecated tokens to migrate your position"));
        } else if !is_position_migrated {
            // if the position is not migrated, but bond amount is zero, it means it's a new position, so store it as migrated
            store_is_migrated(storage, asset_token, staker_addr)?;
        }
    }

    let pool_index = if is_short {
        pool_info.short_reward_index
    } else {
        pool_info.reward_index
    };

    // Withdraw reward to pending reward; before changing share
    before_share_change(pool_index, &mut reward_info)?;

    // Increase total short or bond amount
    if is_short {
        pool_info.total_short_amount += amount;
    } else {
        pool_info.total_bond_amount += amount;
    }

    reward_info.bond_amount += amount;

    rewards_store(storage, staker_addr, is_short).save(asset_token.as_slice(), &reward_info)?;
    store_pool_info(storage, asset_token, &pool_info)?;

    Ok(())
}

fn _decrease_bond_amount(
    storage: &mut dyn Storage,
    staker_addr: &CanonicalAddr,
    asset_token: &CanonicalAddr,
    amount: Uint128,
    is_short: bool,
) -> StdResult<CanonicalAddr> {
    let mut pool_info: PoolInfo = read_pool_info(storage, asset_token)?;
    let mut reward_info: RewardInfo =
        rewards_read(storage, staker_addr, is_short).load(asset_token.as_slice())?;

    if reward_info.bond_amount < amount {
        return Err(StdError::generic_err("Cannot unbond more than bond amount"));
    }

    // if the lp token was migrated, and the user did not close their position yet, cap the reward at the snapshot
    let should_migrate = !read_is_migrated(storage, asset_token, staker_addr)
        && !is_short
        && pool_info.migration_params.is_some();
    let (pool_index, staking_token) = if is_short {
        (
            pool_info.short_reward_index,
            pool_info.staking_token.clone(),
        ) // actually not used later
    } else if should_migrate {
        let migraton_params = pool_info.migration_params.clone().unwrap();
        (
            migraton_params.index_snapshot,
            migraton_params.deprecated_staking_token,
        )
    } else {
        (pool_info.reward_index, pool_info.staking_token.clone())
    };

    // Distribute reward to pending reward; before changing share
    before_share_change(pool_index, &mut reward_info)?;

    // Decrease total short or bond amount
    if is_short {
        pool_info.total_short_amount = pool_info.total_short_amount.checked_sub(amount)?;
    } else if !should_migrate {
        // if it should migrate, we dont need to decrease from the current total bond amount
        pool_info.total_bond_amount = pool_info.total_bond_amount.checked_sub(amount)?;
    }

    // Update rewards info
    reward_info.bond_amount = reward_info.bond_amount.checked_sub(amount)?;

    if reward_info.bond_amount.is_zero() && should_migrate {
        store_is_migrated(storage, asset_token, staker_addr)?;
    }

    if reward_info.pending_reward.is_zero() && reward_info.bond_amount.is_zero() {
        rewards_store(storage, staker_addr, is_short).remove(asset_token.as_slice());
    } else {
        rewards_store(storage, staker_addr, is_short).save(asset_token.as_slice(), &reward_info)?;
    }

    // Update pool info
    store_pool_info(storage, asset_token, &pool_info)?;

    Ok(staking_token)
}
